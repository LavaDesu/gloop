#![feature(array_methods)]
#![feature(iter_intersperse)]

mod commands;
#[macro_use] mod macros;
use std::{env, path::PathBuf};

use serenity::async_trait;
use serenity::model::application::interaction::Interaction;
use serenity::model::{id::GuildId, gateway::Ready};
use serenity::prelude::*;
use sqlx::{sqlite::SqlitePoolOptions, Sqlite, Pool, migrate::Migrator};
use tokio::try_join;
use tracing::{error, info, warn, trace};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

struct Database;

impl TypeMapKey for Database {
    type Value = Pool<Sqlite>;
}

struct Handler;
#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, int: Interaction) {
        if let Interaction::ApplicationCommand(cmd) = int {
            use commands::*;

            trace!("Received interaction: {:#?}", cmd);
            let run = cmdmatch!(ctx, cmd, [
                bet,
                bet_admin_stopper["Stop accepting bets"],
                bet_admin_ender["End and finalise bets"],
                buttontest,
                leaderboards,
                profile["koins"],
            ]);

            if let Err(why) = run
            {
                warn!("Cannot respond to slash command: {} {}", why, why.backtrace());
            }
        }
    }

    async fn ready(&self, ctx: Context, ready: Ready) {
        info!("Hello from {}#{:04}", ready.user.name, ready.user.discriminator);

        let guild_id = GuildId(
            env::var("BLOB_DEV_GUILD")
                .expect("Expected BLOB_DEV_GUILD in environment")
                .parse()
                .expect("BLOB_DEV_GUILD must be an integer"),
        );

        use commands::*;
        let gcmds = GuildId::set_application_commands(&guild_id, &ctx.http, |builder| cmdcreate!(builder, [
            bet,
            bet_admin_stopper,
            bet_admin_ender,
            buttontest,
            leaderboards,
            profile,
        ]))
        .await;

        trace!("Slash commands: {:#?}", gcmds);
    }
}

fn setup_tracing() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(fmt::layer()
              .compact()
              .with_span_events(FmtSpan::CLOSE | FmtSpan::NEW))
        .with(EnvFilter::builder()
              .parse("warn,gloop=info")?
              //.parse("info")?
        )
        .init();

    Ok(())
}

async fn setup_db(url: String) -> anyhow::Result<Pool<Sqlite>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&url).await?;

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

    let (db,) = try_join!(setup_db(db_url))?;

    let mut client = Client::builder(token, GatewayIntents::empty())
        .event_handler(Handler)
        .await
        .expect("Error creating client");

    {
        let mut data = client.data.write().await;
        data.insert::<Database>(db);
    }

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
