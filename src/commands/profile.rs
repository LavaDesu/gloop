use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::interaction::InteractionResponseType;

use crate::Database;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::DeferredChannelMessageWithSource)
            .interaction_response_data(|m| m.ephemeral(true))
    })
    .await?;

    let discord_id: i64 = int.user.id.into();
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();
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
    drop(data);

    int.create_followup_message(&ctx.http, |resp| {
        resp.content(format!("You have {} koins", res.coins))
    })
    .await?;

    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.name("koins")
        .description("Check your balance of Cambodia Osu Cup Koins")
}
