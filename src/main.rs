#![feature(array_methods)]
#![feature(iter_intersperse)]

#[macro_use]
extern crate tracing;
#[macro_use]
mod macros;

mod commands;
use std::{env, path::PathBuf};

use commands::snipe;
use serenity::async_trait;
use serenity::model::application::interaction::Interaction;
use serenity::model::prelude::{MessageId, ChannelId, MessageUpdateEvent};
use serenity::model::{gateway::Ready, id::GuildId, channel::Message};
use serenity::prelude::*;
use sqlx::{migrate::Migrator, sqlite::SqlitePoolOptions, Pool, Sqlite};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

struct Database;

impl TypeMapKey for Database {
    type Value = Pool<Sqlite>;
}

struct Handler;
#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        snipe::recv_msg(ctx, msg).await;
    }
    async fn message_delete(&self, ctx: Context, channel_id: ChannelId, msg_id: MessageId, _: Option<GuildId>) {
        snipe::recv_msg_delete(ctx, channel_id, msg_id).await;
    }
    async fn message_update(&self, ctx: Context, event: MessageUpdateEvent) {
        snipe::recv_msg_update(ctx, event).await;
    }

    async fn interaction_create(&self, ctx: Context, int: Interaction) {
        if let Interaction::ApplicationCommand(cmd) = int {
            use commands::*;

            trace!("Received interaction: {:#?}", cmd);
            let run = cmdmatch!(ctx, cmd, [
                bet,
                bet_admin_stopper["Stop accepting bets"],
                bet_admin_ender["End and finalise bets"],
                leaderboards,
                profile["koins"],
                snipe,
            ]);

            if let Err(why) = run {
                warn!(
                    "Slash command {} failed: {}\n{}",
                    cmd.data.name,
                    why,
                    why.backtrace()
                );
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            "Hello from {}#{:04}",
            ready.user.name, ready.user.discriminator
        );

        // [unwrap] unwrappable because we've already checked for it in main()
        // only way this would fail is if we change this env in this program anywhere,
        // which I don't think will ever be a thing
        let guild_id = GuildId(env::var("BLOB_DEV_GUILD").unwrap().parse().unwrap());

        use commands::*;
        let gcmds = GuildId::set_application_commands(&guild_id, &ctx.http, |builder| {
            cmdcreate!(builder, [
                bet,
                bet_admin_stopper,
                bet_admin_ender,
                leaderboards,
                profile,
                snipe,
            ])
        })
        .await;

        trace!("Slash commands: {:#?}", gcmds);
    }
}

fn setup_tracing() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .compact()
                .with_span_events(FmtSpan::CLOSE | FmtSpan::NEW),
        )
        .with(
            EnvFilter::builder()
                .parse("warn,gloop=info")?
                //.parse("info")?
        )
        .init();

    Ok(())
}

async fn setup_db(url: String) -> anyhow::Result<Pool<Sqlite>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await?;

    let migration_path: PathBuf;

    #[cfg(debug_assertions)]
    {
        let crate_dir = env::var("CARGO_MANIFEST_DIR")?;
        migration_path = std::path::Path::new(&crate_dir).join("./migrations");
    }
    #[cfg(not(debug_assertions))]
    {
        migration_path = std::env::current_dir()?.join("./migrations");
    }

    Migrator::new(migration_path).await?.run(&pool).await?;

    Ok(pool)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing()?;

    let token = env::var("BLOB_TOKEN").expect("Missing BLOB_TOKEN");
    let db_url = env::var("DATABASE_URL").expect("Missing DATABASE_URL");
    env::var("BLOB_DEV_GUILD")
        .expect("Missing BLOB_DEV_GUILD")
        .parse::<u64>()
        .expect("BLOB_DEV_GUILD must be a u64");

    let mut client = Client::builder(token, GatewayIntents::GUILD_MESSAGES | GatewayIntents::MESSAGE_CONTENT)
        .event_handler(Handler)
        .await
        .expect("Error creating client");

    let db = setup_db(db_url).await?;
    let mut data = client.data.write().await;
    data.insert::<Database>(db);
    drop(data);

    snipe::init_state(&client).await;

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
