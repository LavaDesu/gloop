use std::sync::Arc;
use std::time::Duration;

use serenity::builder::{CreateApplicationCommand, CreateComponents, CreateEmbed};
use serenity::collector::CollectModalInteraction;
use serenity::futures::StreamExt;
use serenity::model::id::{ChannelId, MessageId, UserId};
use serenity::model::prelude::command::CommandOptionType;
use serenity::model::prelude::component::{ActionRowComponent, InputTextStyle, ButtonStyle};
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::interaction::message_component::MessageComponentInteraction;
use serenity::model::prelude::interaction::modal::ModalSubmitInteraction;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::prelude::Message;
use serenity::model::Permissions;
use serenity::prelude::*;
use serenity::utils::Colour;
use sqlx::{Pool, Sqlite};
use tokio::sync::oneshot::{self, Sender};
use tracing::Instrument;

use crate::Database;

pub struct CtxState;

impl TypeMapKey for CtxState {
    type Value = BetData;
}

type TeamNames = [String; 2];

#[derive(Clone)]
pub struct BetData {
    pub ender: Arc<Mutex<Option<Sender<Option<bool>>>>>,
    pub stopper: Arc<Mutex<Option<Sender<()>>>>,
    pub msg: (MessageId, ChannelId),
    pub blacklist: Vec<u64>,
    pub teams: TeamNames,
}

async fn calc_payout(
    db: &Pool<Sqlite>,
    bet_id: i64,
) -> anyhow::Result<([f64; 2], [i64; 2], [i64; 2])> {
    let query = sqlx::query!(
        r#"
            SELECT target, bet_placed
            FROM bets_events
            WHERE bet = $1
        "#,
        bet_id
    )
    .fetch_all(db)
    .await?;

    let mut totals = [0, 0];
    let mut bets = [0, 0];

    for row in query {
        if row.target {
            totals[1] += row.bet_placed;
            bets[1] += 1;
        } else {
            totals[0] += row.bet_placed;
            bets[0] += 1;
        }
    }

    let red = totals[0] as f64;
    let blu = totals[1] as f64;

    let mut red_mult = blu / red;
    if !red_mult.is_normal() {
        red_mult = 0.0;
    }
    let mut blu_mult = red / blu;
    if !blu_mult.is_normal() {
        blu_mult = 0.0;
    }
    Ok(([1.0 + red_mult, 1.0 + blu_mult], totals, bets))
}

async fn build_embed(
    db: &Pool<Sqlite>,
    bet_id: i64,
    team_names: &TeamNames,
) -> anyhow::Result<CreateEmbed> {
    let (payout, totals, bets) = calc_payout(db, bet_id).await?;

    let mut embd = CreateEmbed::default();
    embd.title(format!("Team {} vs Team {}", team_names[0], team_names[1]))
        .description("Predict and bet on the match outcome")
        .colour(Colour(0x00FF00))
        .fields([0, 1].map(|i| (
            &team_names[i],
            format!(
                "Bets: {}\nPool: {} koins\nPayout: x{:.2}",
                bets[i], totals[i], payout[i]
            ),
            true,
        )));
    Ok(embd)
}

pub fn build_components(team_names: &TeamNames) -> CreateComponents {
    let mut comp = CreateComponents::default();
    comp.create_action_row(|roww| {
        roww.create_button(|butn| {
                butn.custom_id("betone")
                    .label(format!("Bet for Team {}", team_names[0]))
                    .style(ButtonStyle::Danger)
            })
            .create_button(|butn| {
                butn.custom_id("bettwo")
                    .label(format!("Bet for Team {}", team_names[1]))
            })
    });
    comp
}

async fn finalise_bet(
    ctx: &Context,
    int: Arc<ModalSubmitInteraction>,
    initial_id: &str,
) -> anyhow::Result<()> {
    let amnt = &int.data.components[0].components[0];

    if let ActionRowComponent::InputText(e) = amnt {
        let e = e
            .value
            .parse::<u32>()
            .ok()
            .and_then(|e| if e > 0 { Some(e) } else { None });
        if let Some(amnt) = e {
            data_scope!(ctx, db = Database, state = CtxState, {
                let success = db_setbet(
                    db,
                    int.message.as_ref().unwrap().id,
                    int.user.id,
                    amnt,
                    initial_id == "bettwo",
                )
                .await?;
                if !success {
                    intr_emsg!(int, ctx, "You don't have enough koins to bet this much").await?;
                    return Ok(());
                }

                intr_emsg!(int, ctx, format!(
                    "You've bet {}. Note that payout may change as more people start putting bets.",
                    amnt
                ))
                .await?;

                let embed = build_embed(db, state.msg.0.into(), &state.teams).await?;
                state.msg.1.edit_message(&ctx.http, state.msg.0, |d| d.set_embed(embed)).await?;
            });
        } else {
            intr_emsg!(int, ctx, "Failed to parse bet amount (are you sure it's a valid, positive, no-decimal number?)").await?;
        }
    }

    Ok(())
}

async fn prompt_bet(
    ctx: &Context,
    int: Arc<MessageComponentInteraction>,
    msg: MessageId,
) -> anyhow::Result<()> {
    let coins = data_scope!(ctx, db = Database, state = CtxState, {
        // this is disgusting lol
        if state.blacklist.iter().any(|e| {
            e == int.user.id.as_u64()
            || int.member
                .as_ref()
                .unwrap()
                .roles
                .iter()
                .any(|r| r.as_u64() == e)
        }) {
            intr_emsg!(int, ctx, "You're not allowed to bet in this match!").await?;
            return Ok(());
        }
        let discord_id: i64 = int.user.id.into();
        let msg_id: i64 = msg.into();
        let check = sqlx::query!(
            r#"
                SELECT discord_id
                FROM bets_events
                WHERE bet = $1
                AND discord_id = $2
                LIMIT 1
            "#,
            msg_id,
            discord_id
        )
        .fetch_optional(db)
        .await?
        .is_some();

        if check {
            intr_emsg!(int, ctx, "You've already set a bet!").await?;
            return Ok(());
        }

        sqlx::query!(
            "
                INSERT OR IGNORE INTO currency (discord_id, coins)
                VALUES ($1, $2)
            ",
            discord_id,
            1000
        )
        .execute(db)
        .await?;
        let coins = sqlx::query!(
            "
                SELECT coins
                FROM currency
                WHERE discord_id = $1
                LIMIT 1
            ",
            discord_id
        )
        .fetch_one(db)
        .await?;

        coins.coins
    });

    let cid = format!("betamnt{}", int.id);
    let clone = cid.clone();
    int.create_interaction_response(ctx, |resp| {
        resp.kind(InteractionResponseType::Modal)
            .interaction_response_data(|data| {
                data.custom_id(clone)
                    .title("Set your bet amount")
                    .components(|cmp| {
                        cmp.create_action_row(|row| {
                            row.create_input_text(|text| {
                                text.custom_id("betinput")
                                    .label("Bet amount")
                                    .placeholder(format!("You currently have {} koins", coins))
                                    .style(InputTextStyle::Short)
                            })
                        })
                    })
            })
    })
    .await?;

    let modal_int = CollectModalInteraction::new(ctx)
        .message_id(msg)
        .timeout(Duration::from_secs(60))
        .author_id(int.user.id)
        .filter(move |c| c.data.custom_id == cid)
        .await;

    if let Some(modal_int) = modal_int {
        let id = int.data.custom_id.as_str();
        finalise_bet(ctx, modal_int, id).await?;
    }

    Ok(())
}

async fn db_setbet(
    db: &Pool<Sqlite>,
    msg: MessageId,
    user: UserId,
    amnt: u32,
    team: bool,
) -> anyhow::Result<bool> {
    let msg_id: i64 = msg.into();
    let discord_id: i64 = user.into();

    let res = sqlx::query!(
        "
            SELECT coins
            FROM currency
            WHERE discord_id = $1
            LIMIT 1
        ",
        discord_id
    )
    .fetch_one(db)
    .await?;

    if res.coins < amnt.into() {
        return Ok(false);
    }

    let datetime = chrono::offset::Utc::now();
    sqlx::query!(
        "
            INSERT INTO bets_events
                (discord_id, target, time, bet_placed, bet)
            VALUES
                ($1, $2, $3, $4, $5)
        ",
        discord_id,
        team,
        datetime,
        amnt,
        msg_id
    )
    .execute(db)
    .await?;

    let subamnt = amnt - std::cmp::min(amnt / 10, 100);
    sqlx::query!(
        "
            UPDATE currency
            SET coins = coins - $1
            WHERE discord_id = $2
        ",
        subamnt,
        discord_id
    )
    .execute(db)
    .await?;

    Ok(true)
}

async fn send_user(
    ctx: &Context,
    user: UserId,
    embed: CreateEmbed,
) -> Result<Message, SerenityError> {
    user.create_dm_channel(ctx)
        .await?
        .send_message(ctx, |c| c.set_embed(embed))
        .await
}

async fn db_payout(
    ctx: &Context,
    db: &Pool<Sqlite>,
    msg: &Message,
    winner: Option<bool>,
) -> anyhow::Result<()> {
    let msg_id: i64 = msg.id.into();
    let payout = calc_payout(db, msg_id).await?.0;
    let events = sqlx::query!(
        r#"
            SELECT discord_id, target, bet_placed
            FROM bets_events
            WHERE bet = $1
        "#,
        msg_id
    )
    .fetch_all(db)
    .await?;
    let mut msgq = vec![];
    for row in events {
        let mut embd = CreateEmbed::default();
        embd.title("You got mail!");
        if let Some(winner) = winner {
            if row.target == winner {
                let payout = payout[usize::from(winner)];
                let coins = row.bet_placed as f64 * payout;
                sqlx::query!(
                    "
                        UPDATE currency
                        SET coins = coins + $1
                        WHERE discord_id = $2
                    ",
                    coins,
                    row.discord_id
                )
                .execute(db)
                .await?;

                let coins = coins.round() as i64;
                embd.colour(Colour(0x00FF00))
                    .description(format!(
                        "You won {} koins from [this bet]({})",
                        coins,
                        msg.link()
                    ));
            } else {
                embd.colour(Colour::RED)
                    .description(format!(
                        "You lost {} koins from [this bet]({})",
                        row.bet_placed,
                        msg.link()
                    ));
            }
        } else {
            sqlx::query!(
                "
                    UPDATE currency
                    SET coins = coins + $1
                    WHERE discord_id = $2
                ",
                row.bet_placed,
                row.discord_id
            )
            .execute(db)
            .await?;
            embd.colour(Colour(0))
                .description(format!(
                    "You've been refunded {} koins from [this bet]({})",
                    row.bet_placed,
                    msg.link()
                ));
        }

        let msg = send_user(ctx, UserId(row.discord_id as u64), embd);
        msgq.push(msg);
    }

    for f in msgq {
        // discard error if dm unable to be sent (eg. user disabled dms)
        let _ = f.await;
    }

    Ok(())
}

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    if ctx.data.read().await.contains_key::<CtxState>() {
        intr_emsg!(int, ctx, "There's already a bet running!").await?;
        return Ok(());
    }

    let teams = ["red_name", "blue_name"].map(|i| {
        int.data.options
            .iter()
            .find(|o| o.name == i)
            .and_then(|o| o.value.as_ref())
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string()
    });

    let blacklist = int.data.options
        .iter()
        .find(|o| o.name == "blacklist")
        .and_then(|o| o.value.as_ref())
        .and_then(|v| v.as_str())
        .map_or_else(|| Ok(vec![]), |allv| {
            allv.split(",")
                .map(|i| i.parse::<u64>())
                .collect()
        });

    if let Err(_) = blacklist {
        intr_emsg!(int, ctx, "Invalid ID(s) in blacklist").await?;
        return Ok(());
    }

    let blacklist = blacklist.unwrap();

    let msg = int
        .channel_id
        .send_message(&ctx.http, |rmsg| {
            rmsg.add_embed(|embd| {
                embd.title(format!("Team {} vs Team {}", &teams[0], &teams[1]))
                    .description("Predict and bet on the match outcome")
            })
        })
        .await?;
    let mut interaction_stream = msg.await_component_interactions(ctx).build();

    // /* Init state
    let (stop_sender, mut stop_receiver) = oneshot::channel();
    let (end_sender, mut end_receiver) = oneshot::channel();
    let state = BetData {
        ender: Arc::new(Mutex::new(Some(end_sender))),
        stopper: Arc::new(Mutex::new(Some(stop_sender))),
        msg: (msg.id, msg.channel_id),
        blacklist,
        teams,
    };
    ctx.data
        .write()
        .await
        .insert::<CtxState>(state.clone());
    //    Init state */

    // /* Create db bet
    let embed = data_scope!(ctx, db = Database, {
        let msg_id: i64 = msg.id.into();
        let datetime = chrono::offset::Utc::now();
        sqlx::query!(
            r#"
                INSERT INTO bets (msg_id, start_time)
                VALUES ($1, $2)
            "#,
            msg_id,
            datetime
        )
        .execute(db)
        .await?;

        build_embed(db, msg_id, &state.teams).await?
    });
    state.msg.1
        .edit_message(&ctx.http, msg.id, |nmsg| {
            nmsg.set_components(build_components(&state.teams))
                .set_embed(embed)
        })
        .await?;
    //    Create db bet */

    intr_emsg!(int, ctx, "Bet ready").await?;

    let mut end_res = None;
    let mut handles = vec![];
    while let Some(interaction) = tokio::select! {
        v = interaction_stream.next() => v,
        _ = &mut stop_receiver => None,
        e = &mut end_receiver => { end_res = Some(e.unwrap()); None },
    } {
        let ctx = ctx.clone();
        let iid = *interaction.id.as_u64();
        let uid = *interaction.user.id.as_u64();
        let span = info_span!("prompt_bet", iid, uid);

        let handle = tokio::spawn(
            async move {
                if let Err(why) = prompt_bet(&ctx, interaction, msg.id).await {
                    warn!(
                        "Int {} by {} errored: {}\n{}",
                        iid,
                        uid,
                        why,
                        why.backtrace()
                    );
                }
            }
            .instrument(span),
        );
        handles.push(handle);
    }

    // If not ended; only stopped
    if end_res.is_none() {
        let mut embed = data_scope!(ctx, db = Database, {
            build_embed(db, msg.id.into(), &state.teams).await?
        });

        embed.colour(Colour::ORANGE);
        embed.description("Bets are no longer being accepted. Sit tight for results!");
        msg.channel_id
            .edit_message(&ctx, msg.id, |emsg| {
                emsg.set_components(CreateComponents::default())
                    .set_embed(embed)
            })
            .await?;

        end_res = Some(end_receiver.await?);
    }

    for handle in handles {
        handle.abort();
    }

    let end_res = end_res.unwrap();
    let mut embed = data_scope!(ctx, db = Database, {
        let datetime = chrono::offset::Utc::now();
        let mid: i64 = msg.id.into();

        sqlx::query!(
            r#"
                UPDATE bets
                SET stop_time = CASE WHEN stop_time IS NULL THEN $1 ELSE stop_time END,
                    end_time = $1,
                    blue_win = $2
                WHERE msg_id = $3
            "#,
            datetime,
            end_res,
            mid
        )
        .execute(db)
        .await?;

        build_embed(db, msg.id.into(), &state.teams).await?
    });

    if let Some(end_res) = end_res {
        embed.colour(if end_res { Colour::BLUE } else { Colour::RED });
        embed.description(format!("Bets have concluded.\nThe winner is **{}**!", state.teams[usize::from(end_res)]));
    } else {
        embed.colour(Colour(0));
        embed.description("Match is cancelled. Bets have been refunded.");
    }

    if let Err(why) = msg.channel_id
        .edit_message(&ctx, msg.id, |emsg| {
            emsg.set_components(CreateComponents::default())
                .set_embed(embed)
        })
        .await
    {
        warn!("Failed to edit bet message for {}: {}", msg.id.as_u64(), why);
    }

    data_scope!(ctx, db = Database, {
        db_payout(ctx, db, &msg, end_res).await?;
    });

    ctx.data.write().await.remove::<CtxState>();
    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.name("bet")
        .description("bet deez nuts")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .create_option(|optn| {
            optn.name("red_name")
                .description("Red team name")
                .kind(CommandOptionType::String)
                .required(true)
        })
        .create_option(|optn| {
            optn.name("blue_name")
                .description("Blue team name")
                .kind(CommandOptionType::String)
                .required(true)
        })
        .create_option(|optn| {
            optn.name("blacklist")
                .description("Blacklist certain roles or members from placing bets, separated by commas")
                .kind(CommandOptionType::String)
                .required(false)
        })
}
