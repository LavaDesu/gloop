use std::sync::Arc;
use std::time::Duration;

use serenity::builder::{CreateEmbed, CreateApplicationCommand, CreateComponents};
use serenity::collector::CollectModalInteraction;
use serenity::futures::StreamExt;
use serenity::model::Permissions;
use serenity::model::id::{MessageId, ChannelId, UserId};
use serenity::model::prelude::Message;
use serenity::model::prelude::command::CommandOptionType;
use serenity::model::prelude::component::{ActionRowComponent, InputTextStyle};
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::interaction::message_component::MessageComponentInteraction;
use serenity::model::prelude::interaction::modal::ModalSubmitInteraction;
use serenity::prelude::*;
use serenity::utils::Colour;
use sqlx::{Pool, Sqlite};
use tokio::sync::oneshot::{self, Sender};
use tracing::{info_span, Instrument};

use crate::Database;

pub struct CtxState;

impl TypeMapKey for CtxState {
    type Value = Arc<RwLock<BetState>>;
}

type TeamNames = [String; 2];

pub struct BetState {
    pub ender: Option<Sender<bool>>,
    pub stopper: Option<Sender<()>>,
    pub msg: (MessageId, ChannelId),
    pub teams: TeamNames
}

async fn calc_payout(db: &Pool<Sqlite>, bet_id: i64) -> anyhow::Result<([f64; 2], [i32; 2])> {
    let query = sqlx::query!(
        r#"
            SELECT
                SUM(CASE WHEN target = 0 THEN 1 ELSE 0 END) as "red!: i32",
                SUM(CASE WHEN target = 1 THEN 1 ELSE 0 END) as "blu!: i32"
            FROM bets_events
            WHERE bet = $1
        "#,
        bet_id
    ).fetch_one(db).await?;

    let red = query.red as f64;
    let blu = query.blu as f64;

    let mut red_mult = blu / red;
    if !red_mult.is_normal() { red_mult = 0.0; }
    let mut blu_mult = red / blu;
    if !blu_mult.is_normal() { blu_mult = 0.0; }
    Ok((
        [
            1.0 + red_mult,
            1.0 + blu_mult,
        ],
        [query.red, query.blu]
    ))
}

async fn build_embed(db: &Pool<Sqlite>, bet_id: i64, team_names: &TeamNames) -> anyhow::Result<CreateEmbed> {
    let (payout, bets) = calc_payout(db, bet_id).await?;

    let mut embd = CreateEmbed::default();
    embd.title(format!("Team {} vs Team {}", team_names[0], team_names[1]))
        .description("Predict and bet on the match outcome")
        .fields([
            (&team_names[0], format!("Bets: {}\nPayout: x{:.2}", bets[0], payout[0]), true),
            (&team_names[1], format!("Bets: {}\nPayout: x{:.2}", bets[1], payout[1]), true)
        ]);
    Ok(embd)
}

pub fn build_components(team_names: &TeamNames) -> CreateComponents {
    let mut comp = CreateComponents::default();
    comp.create_action_row(|roww| {
        roww.create_button(|butn| {
                butn.custom_id("betone")
                    .label(format!("Bet for Team {}", team_names[0]))
            })
            .create_button(|butn| {
                butn.custom_id("bettwo")
                    .label(format!("Bet for Team {}", team_names[1]))
            })
    });
    comp
}


async fn finalise_bet(ctx: &Context, int: Arc<ModalSubmitInteraction>, initial_id: &str) -> anyhow::Result<()> {
    let amnt = &int.data.components[0].components[0];
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();
    let state_mutex = data.get::<CtxState>().unwrap();

    if let ActionRowComponent::InputText(e) = amnt {
        if let Ok(amnt) = e.value.parse::<u32>() {
            let state = state_mutex.read().await;
            let success = db_setbet(db, int.message.as_ref().unwrap().id, int.user.id, amnt, initial_id == "bettwo").await?;
            if !success {
                int.create_interaction_response(&ctx.http, |resp| {
                    resp.kind(InteractionResponseType::ChannelMessageWithSource)
                        .interaction_response_data(|data| {
                            data.content("You don't have enough koins to bet this much")
                                .ephemeral(true)
                        })
                }).await?;
                return Ok(());
            }

            int.create_interaction_response(&ctx.http, |resp| {
                resp.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| {
                        data.content(format!("You've bet {}. Note that payout may change as more people start putting bets.", amnt))
                            .ephemeral(true)
                    })
            }).await?;

            let embed = build_embed(db, state.msg.0.into(), &state.teams).await?;
            state.msg.1.edit_message(&ctx.http, state.msg.0, |d| d.set_embed(embed)).await?;
        } else {
            int.create_interaction_response(&ctx.http, |resp| {
                resp.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| {
                        data.content("Failed to parse bet amount (are you sure it's a valid, positive, no-decimal number?)")
                            .ephemeral(true)
                    })
            }).await?;
        }
    }

    Ok(())
}

async fn prompt_bet(ctx: &Context, int: Arc<MessageComponentInteraction>, msg: MessageId) -> anyhow::Result<()> {
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();

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
    ).fetch_optional(db).await?.is_some();

    if check {
        int.create_interaction_response(&ctx.http, |resp| {
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|resd| {
                    resd.content("You've already set a bet!")
                        .ephemeral(true)
                })
        }).await?;
        return Ok(());
    }

    sqlx::query!(
        "
            INSERT OR IGNORE INTO currency (discord_id, coins)
            VALUES ($1, $2)
        ",
        discord_id,
        1000
    ).execute(db).await?;
    let coins = sqlx::query!(
        "
            SELECT coins
            FROM currency
            WHERE discord_id = $1
            LIMIT 1
        ",
        discord_id
    ).fetch_one(db).await?;
    drop(data);

    let cid = format!("betamnt{}", int.id);
    let clone = cid.clone();
    int.create_interaction_response(ctx, |resp| {
        resp.kind(InteractionResponseType::Modal)
            .interaction_response_data(|data| {
                data.custom_id(clone)
                    .title("Set your bet amount")
                    .components(|c| c.create_action_row(|roww| {
                        roww.create_input_text(|text| {
                            text.custom_id("betinput")
                                .label("Bet amount")
                                .placeholder(format!("You currently have {} koins", coins.coins))
                                .style(InputTextStyle::Short)
                        })
                    }))
            })
    }).await?;

    let modal_int = CollectModalInteraction::new(ctx).message_id(msg)
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

async fn db_setbet(db: &Pool<Sqlite>, msg: MessageId, user: UserId, amnt: u32, team: bool) -> anyhow::Result<bool> {
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
    ).fetch_one(db).await?;

    if res.coins < amnt.into() {
        return Ok(false)
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
    ).execute(db).await?;

    sqlx::query!(
        "
            UPDATE currency
            SET coins = coins - $1
            WHERE discord_id = $2
        ",
        amnt,
        discord_id
    ).execute(db).await?;

    Ok(true)
}

async fn send_user(ctx: &Context, user: UserId, embed: CreateEmbed) -> Result<Message, SerenityError> {
    user.create_dm_channel(ctx).await?.send_message(ctx, |c| c.set_embed(embed)).await
}

async fn db_payout(ctx: &Context, db: &Pool<Sqlite>, msg: &Message, winner: bool) -> anyhow::Result<()> {
    let msg_id: i64 = msg.id.into();
    let (payout, _) = calc_payout(db, msg_id).await?;
    let payout = payout[usize::from(winner)];
    let events = sqlx::query!(
        r#"
            SELECT discord_id, target, bet_placed
            FROM bets_events
            WHERE bet = $1
        "#,
        msg_id
    ).fetch_all(db).await?;
    let mut msgq = vec![];
    for row in events {
        let mut embd = CreateEmbed::default();
        embd.title("You got mail!");
        if row.target == winner {
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
            embd
                .colour(Colour::from_rgb(0, 255, 0))
                .description(format!("You won {} koins from [this bet]({})", coins, msg.link()));
        } else {
            embd
                .colour(Colour::from_rgb(255, 0, 0))
                .description(format!("You lost {} koins from [this bet]({})", row.bet_placed, msg.link()));
        }

        let msg = send_user(ctx, UserId(row.discord_id as u64), embd);
        msgq.push(msg);
    }

    for f in msgq {
        f.await?;
    }

    Ok(())
}

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    if ctx.data.read().await.contains_key::<CtxState>() {
        int.create_interaction_response(&ctx.http, |resp| {
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|m| m.ephemeral(true).content("There's already a bet running at (msg placeholder)"))
        }).await?;

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

    // /* Init state
    let state = BetState {
        ender: None,
        stopper: None,
        msg: (MessageId(0), ChannelId(0)),
        teams
    };
    let mutex = Arc::new(RwLock::new(state));
    ctx.data.write().await.insert::<CtxState>(Arc::clone(&mutex));
    //    Init state */

    // /* Send messages
    let state = mutex.read().await;
    // Bet message
    let msg = int.channel_id.send_message(&ctx.http, move |rmsg| {
        rmsg.add_embed(|embd|
                embd.title(format!("Team {} vs Team {}", &state.teams[0], &state.teams[1]))
                    .description("Predict and bet on the match outcome")
            )
            .set_components(build_components(&state.teams))
    }).await?;
    mutex.write().await.msg = (msg.id, msg.channel_id);
    let mut interaction_stream = msg.await_component_interactions(ctx).build();

    // Int message
    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|m| m.ephemeral(true).content("sentt"))
    }).await?;
    //    Send messages */

    // /* Create db bet
    {
        let data = ctx.data.read().await;
        let db = data.get::<Database>().unwrap();
        let msg_id: i64 = msg.id.into();
        let datetime = chrono::offset::Utc::now();
        sqlx::query!(
            r#"
                INSERT INTO bets (msg_id, start_time)
                VALUES ($1, $2)
            "#,
            msg_id,
            datetime
        ).execute(db).await?;

        let state = mutex.read().await;
        let embed = build_embed(db, msg_id, &state.teams).await?;
        state.msg.1.edit_message(&ctx.http, msg.id, |d| d.set_embed(embed)).await?;
    }
    //    Create db bet */

    // /* Init oneshots
    let mut state = mutex.write().await;
    let (stop_sender, mut stop_receiver) = oneshot::channel::<()>();
    let (end_sender, mut end_receiver) = oneshot::channel::<bool>();
    state.stopper = Some(stop_sender);
    state.ender = Some(end_sender);
    drop(state);
    //    Init oneshots */

    let mut end_res = None;
    let mut handles = vec![];
    while let Some(interaction) = tokio::select! {
        v = interaction_stream.next() => v,
        _ = &mut stop_receiver => None,
        e = &mut end_receiver => { end_res = Some(e.unwrap()); None },
    } {
        let ctx = ctx.clone();
        let msg = msg.clone();

        let iid = interaction.id.as_u64();
        let uid = interaction.user.id.as_u64();
        let span = info_span!("prompt_bet", iid, uid);
        let handle = tokio::spawn(async move {
            prompt_bet(&ctx, interaction, msg.id).await.unwrap();
        }.instrument(span));
        handles.push(handle);
    }

    // Clear components (after stop/end received)
    msg.channel_id.edit_message(&ctx, msg.id, |m| m.set_components(CreateComponents::default())).await?;

    // If not ended, wait til ended
    if end_res.is_none() {
        end_res = Some(end_receiver.await?);
    }
    for handle in handles {
        handle.abort();
    }

    let mut data = ctx.data.write().await;
    let db = data.get::<Database>().unwrap();
    let datetime = chrono::offset::Utc::now();
    let end_res = end_res.unwrap();
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
    ).execute(db).await?;

    db_payout(ctx, db, &msg, end_res).await?;
    data.remove::<CtxState>();
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
}
