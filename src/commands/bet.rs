use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use rosu_v2::Osu;
use serenity::builder::{CreateEmbed, CreateApplicationCommand, CreateComponents};
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
use tokio::sync::oneshot::error::TryRecvError;
use tokio::sync::oneshot::{self, Sender};
use tokio::time::{interval, MissedTickBehavior};
use tracing::{info_span, Instrument};

use crate::{Database, OsuData};

pub struct CtxState;

impl TypeMapKey for CtxState {
    type Value = Arc<RwLock<BetState>>;
}

pub struct TeamState {
    pub bets: HashMap<UserId, i64>,
    pub bias: f64,
    pub name: String
}

type Teams = [TeamState; 2];

pub struct BetState {
    pub ender: Option<Sender<bool>>,
    pub stopper: Option<Sender<()>>,
    pub msg: (MessageId, ChannelId),
    pub teams: Teams
}

fn calc_payout(teams: &Teams, swap: bool) -> f64 {
    let mut teams = teams.each_ref();
    if swap { teams.reverse(); }
    let bias = 0.0;
    //let bias = teams[0].bias;
    let bets = teams[0].bets.len() as f64;
    let other_bets = teams[1].bets.len() as f64;
    let mut payout = 1.0 + (1.0 + bias) * other_bets / bets;
    if !payout.is_normal() || other_bets == 0.0 {
        payout = 2.0;
    }

    payout
}

fn create_fields(teams: &Teams) -> [(&String, String, bool); 2] {
    let mut swap = false;
    teams.each_ref().map(|team| {
        let bias = team.bias;
        let bets = team.bets.len();
        let payout = calc_payout(teams, swap);
        swap = true;
        //(&team.name, format!("Bias: {:.2}%\nBets: {}\nPayout: x{:.2}", bias * 100.0, bets, payout), true)
        (&team.name, format!("Bets: {}\nPayout: x{:.2}", bets, payout), true)
    })
}

fn build_embed(teams: &Teams) -> CreateEmbed {
    let mut embd = CreateEmbed::default();
    embd.title(format!("Team {} vs Team {}", teams[0].name, teams[1].name))
        .description("Predict and bet on the match outcome")
        .fields(create_fields(teams));
    embd
}

pub fn build_components(teams: &Teams) -> CreateComponents {
    let mut comp = CreateComponents::default();
    comp.create_action_row(|roww| {
        roww.create_button(|butn| {
                butn.custom_id("betone")
                    .label(format!("Bet for Team {}", teams[0].name))
            })
            .create_button(|butn| {
                butn.custom_id("bettwo")
                    .label(format!("Bet for Team {}", teams[1].name))
            })
    });
    comp
}


async fn finalise_bet(ctx: &Context, mutex: Arc<RwLock<BetState>>, int: Arc<ModalSubmitInteraction>, initial_id: &str) -> anyhow::Result<()> {
    let amnt = &int.data.components[0].components[0];

    if let ActionRowComponent::InputText(e) = amnt {
        if let Ok(amnt) = e.value.parse::<u32>() {
            let mut state = mutex.write().await;
            let target = match initial_id {
                "betone" => &mut state.teams[0],
                "bettwo" => &mut state.teams[1],
                _ => panic!("invalid button selected while betting")
            };
            let success = db_setbet(ctx, int.user.id, amnt).await?;
            if !success {
                int.create_interaction_response(&ctx.http, |resp| {
                    resp.kind(InteractionResponseType::ChannelMessageWithSource)
                        .interaction_response_data(|data| {
                            data.content(format!("You don't have enough coins to bet this much"))
                                .ephemeral(true)
                        })
                }).await?;
                return Ok(());
            }
            target.bets.insert(int.user.id, amnt.into());

            int.create_interaction_response(&ctx.http, |resp| {
                resp.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| {
                        data.content(format!("You've bet {}. Note that payout may change as more people start putting bets.", amnt))
                            .ephemeral(true)
                    })
            }).await?;

            let embed = build_embed(&state.teams);
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

async fn prompt_bet(ctx: &Context, mutex: Arc<RwLock<BetState>>, int: Arc<MessageComponentInteraction>, msg: &Message) -> anyhow::Result<()> {
    let state = mutex.read().await;
    if state.teams.iter().any(|team| team.bets.contains_key(&int.user.id)) {
        int.create_interaction_response(&ctx.http, |resp| {
            resp.kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|data| {
                    data.content("You've already set a bet!")
                        .ephemeral(true)
                })
        }).await?;
        return Ok(());
    }
    drop(state);

    let cid = format!("betamnt{}", int.id);
    let clone = cid.clone();
    int.create_interaction_response(&ctx, |resp| {
        resp.kind(InteractionResponseType::Modal)
            .interaction_response_data(|data| {
                data.custom_id(clone)
                    .title("Set your bet amount")
                    .components(|c| c.create_action_row(|roww| {
                        roww.create_input_text(|text| {
                            text.custom_id("betinput")
                                .label("Bet amount")
                                .placeholder("e.g. 100 or 727")
                                .style(InputTextStyle::Short)
                        })
                    }))
            })
    }).await?;

    let modal_int = msg.await_modal_interaction(&ctx.shard)
        .timeout(Duration::from_secs(60))
        .author_id(int.user.id)
        .filter(move |c| c.data.custom_id == cid)
        .await;

    if let Some(modal_int) = modal_int {
        let id = int.data.custom_id.as_str();
        finalise_bet(ctx, mutex, modal_int, id).await?;
    }

    Ok(())
}

// TODO: these sql statements are very very inefficient
async fn db_setbet(ctx: &Context, user: UserId, amnt: u32) -> anyhow::Result<bool> {
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();
    let discord_id = *user.as_u64() as i64;
    sqlx::query!(
        "
            INSERT OR IGNORE INTO currency (discord_id, coins)
            VALUES ($1, $2)
        ",
        discord_id,
        1000
    ).execute(db).await?;

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

async fn db_payout(ctx: &Context, msg: &String, teams: &Teams, winner: bool) -> anyhow::Result<()> {
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();

    let mut msgq = VecDeque::with_capacity(teams.iter().map(|t| t.bets.len()).reduce(|a, b| a + b).unwrap());
    for (uid, bet) in &teams[winner as usize].bets {
        let discord_id = *uid.as_u64() as i64;
        let coins = ((*bet as f64) * calc_payout(teams, winner)) as i64;
        sqlx::query!(
            "
                UPDATE currency
                SET coins = coins + $1
                WHERE discord_id = $2
            ",
            coins,
            discord_id
        )
            .execute(db)
            .await?;

        let mut embd = CreateEmbed::default();
        embd.title("You got mail!")
            .colour(Colour::from_rgb(0, 255, 0))
            .description(format!("You won {} from [this bet]({})", coins, msg));

        let msg = send_user(ctx, uid.clone(), embd);
        msgq.push_back(msg);
    }

    for (uid, bet) in &teams[!winner as usize].bets {
        let mut embd = CreateEmbed::default();
        embd.title("You got mail!")
            .colour(Colour::from_rgb(255, 0, 0))
            .description(format!("You lost {} from [this bet]({})", bet, msg));

        let msg = send_user(ctx, uid.clone(), embd);
        msgq.push_back(msg);
    }

    for f in msgq {
        f.await?;
    }

    Ok(())
}

async fn update_matches(osu: Arc<Osu>, match_id: u32, state: Arc<RwLock<BetState>>) -> anyhow::Result<()> {
    let mut int = interval(Duration::from_secs(10));
    int.set_missed_tick_behavior(MissedTickBehavior::Delay);

    let mut osu_match = osu.osu_match(match_id).await?;
    loop {
        int.tick().await;
        osu_match = osu_match.get_next(&osu).await?;
        if osu_match.drain_games().next() != None {
            if let Some(sender) = state.write().await.stopper.take() {
                sender.send(()).unwrap();
            }
            break
        }
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

    let bias = int.data.options.get(0)
        .and_then(|o| o.value.as_ref())
        .and_then(|v| v.as_f64())
        .unwrap_or(0.5);

    let state = BetState {
        ender: None,
        stopper: None,
        msg: (MessageId(0), ChannelId(0)),
        teams: [
            TeamState { bets: HashMap::new(), bias, name: "Red".to_string() },
            TeamState { bets: HashMap::new(), bias: 1.0 - bias, name: "Blue".to_string() }
        ]
    };
    let mutex = Arc::new(RwLock::new(state));
    ctx.data.write().await.insert::<CtxState>(Arc::clone(&mutex));

    let state = mutex.read().await;
    let msg = int.channel_id.send_message(&ctx.http, move |rmsg| {
        rmsg.set_embed(build_embed(&state.teams))
            .set_components(build_components(&state.teams))
    }).await?;
    mutex.write().await.msg = (msg.id, msg.channel_id);

    let mut interaction_stream = msg.await_component_interactions(&ctx).build();
    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|m| m.ephemeral(true).content("sentt"))
    }).await?;

    let mut state = mutex.write().await;
    let (sender, mut stop_receiver) = oneshot::channel::<()>();
    state.stopper = Some(sender);
    let (sender, mut end_receiver) = oneshot::channel::<bool>();
    state.ender = Some(sender);
    drop(state);

    let mut match_handle = None;
    if let Some(match_id) = int.data.options.get(0)
        .and_then(|o| o.value.as_ref())
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok()) {
        let data = ctx.data.read().await;
        let osu = data.get::<OsuData>().unwrap();
        let c_osu = Arc::clone(osu);
        let c_mutex = Arc::clone(&mutex);
        match_handle = Some(tokio::spawn(async move { update_matches(c_osu, match_id, c_mutex).await }));
    }

    let mut end_res = None;
    let mut handles = vec![];
    while let Some(interaction) = tokio::select! {
        v = interaction_stream.next() => v,
        _ = &mut stop_receiver => None,
        e = &mut end_receiver => { end_res = Some(e.unwrap()); None },
    } {
        let ctx = ctx.clone();
        let mutex = Arc::clone(&mutex);
        let msg = msg.clone();

        let iid = interaction.id.as_u64();
        let span = info_span!("func", iid);
        let handle = tokio::spawn(async move {
            prompt_bet(&ctx, mutex, interaction, &msg).await.unwrap()
        }.instrument(span));
        handles.push(handle);
    }

    msg.channel_id.edit_message(&ctx, msg.id, |m| m.set_components(CreateComponents::default())).await?;

    if end_res.is_none() {
        let ret = end_receiver.try_recv();
        let ret = match ret {
            Err(TryRecvError::Empty) => end_receiver.await?,
            // XXX: BUG: TODO
            Err(TryRecvError::Closed) => true,
            Ok(e) => e
        };
        end_res = Some(ret);

        for handle in handles {
            handle.await?;
        }
    } else {
        for handle in handles {
            handle.abort();
        }
    }

    if let Some(handle) = match_handle.take() { handle.abort(); }
    let state = mutex.read().await;
    db_payout(ctx, &msg.link(), &state.teams, end_res.unwrap()).await?;
    drop(state);
    ctx.data.write().await.remove::<CtxState>();

    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.name("bet")
        .description("bet deez nuts")
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .create_option(|optn| {
            optn.name("match_id")
                .description("Multiplayer match ID")
                .kind(CommandOptionType::Integer)
                .min_int_value(1)
                .required(false)
        })
        .create_option(|optn| {
            optn.name("bias")
                .description("bias for first team (default = 0.5 = fair/no bias)")
                .kind(CommandOptionType::Number)
                .min_number_value(0.0)
                .max_number_value(1.0)
        })
}
