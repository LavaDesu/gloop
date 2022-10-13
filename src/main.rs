#![feature(array_methods)]

mod commands;
#[macro_use] mod macros;
use std::sync::Arc;
use std::{env, path::{PathBuf, Path}};

use rosu_v2::Osu;
use serenity::async_trait;
use serenity::model::application::interaction::Interaction;
use serenity::model::{id::GuildId, gateway::Ready};
use serenity::prelude::*;
use sqlx::{sqlite::SqlitePoolOptions, Sqlite, Pool, migrate::Migrator};
use tokio::try_join;
use tracing::{error, info, warn, trace};
use tracing_subscriber::filter::Directive;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

struct Database;
struct OsuData;

impl TypeMapKey for Database {
    type Value = Pool<Sqlite>;
}
impl TypeMapKey for OsuData {
    type Value = Arc<Osu>;
}

struct Handler;
#[async_trait]
impl EventHandler for Handler {
    async fn interaction_create(&self, ctx: Context, int: Interaction) {
        if let Interaction::ApplicationCommand(cmd) = int {
            trace!("Received interaction: {:#?}", cmd);

            let run = cmdmatch!(ctx, cmd, [
                bet,
                bet_admin_stopper("Stop accepting bets"),
                bet_admin_ender("End and finalise bets"),
                buttontest,
                profile("coins"),
            ]);

            if let Err(why) = run
            {
                warn!("Cannot respond to slash command: {}", why.backtrace());
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

        let gcmds = GuildId::set_application_commands(&guild_id, &ctx.http, |builder| cmdcreate!(builder, [
            bet,
            bet_admin_stopper,
            bet_admin_ender,
            buttontest,
            profile,
        ]))
        .await;

        trace!("Slash commands: {:#?}", gcmds);
    }
}

fn setup_tracing() -> anyhow::Result<()> {
    /*
    let offset = UtcOffset::current_local_offset()?;
    let timer = OffsetTime::new(offset, format_description!("[hour]:[minute]:[second]"));
    tracing_subscriber::registry()
        .with(fmt::layer()
              .compact()
              .with_timer(timer))
        .with(EnvFilter::builder()
              .with_default_directive("sinuous=info".parse()?)
              .from_env()?
        )
        .init();
    */

    tracing_subscriber::registry()
        .with(fmt::layer().compact())
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
        migration_path = Path::new(&crate_dir).join("./migrations");
    }
    #[cfg(not(debug_assertions))]
    {
        migration_path = std::env::current_exe()?.join("./migrations");
    }

    Migrator::new(migration_path).await?.run(&pool).await?;

    Ok(pool)
}

async fn setup_osu(id: u64, secret: String) -> anyhow::Result<Osu> {
    Ok(Osu::new(id, secret).await?)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_tracing()?;

    let token = env::var("BLOB_TOKEN").expect("Missing BLOB_TOKEN");
    let db_url = env::var("DATABASE_URL").expect("Missing DATABASE_URL");
    let osu_id = env::var("BLOB_ID").ok().and_then(|i| i.parse::<u64>().ok()).expect("Missing or invalid BLOB_ID");
    let osu_secret = env::var("BLOB_SECRET").expect("Missing BLOB_SECRET");

    let (db, osu) = try_join!(setup_db(db_url), setup_osu(osu_id, osu_secret))?;

    let mut client = Client::builder(token, GatewayIntents::empty())
        .event_handler(Handler)
        .await
        .expect("Error creating client");

    {
        let mut data = client.data.write().await;
        data.insert::<Database>(db);
        data.insert::<OsuData>(Arc::new(osu));
    }

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}
