use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;

use crate::Database;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::DeferredChannelMessageWithSource)
            .interaction_response_data(|m| m.ephemeral(true))
    }).await?;

    let discord_id = *int.user.id.as_u64() as i64;
    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();
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
    )
        .fetch_one(db)
        .await?;
    drop(data);

    int.create_followup_message(&ctx.http, |resp| {
        resp.content(format!("You have {} coins", res.coins))
    }).await?;

    Ok(())
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command.name("coins").description("see coins")
}

