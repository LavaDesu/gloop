use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::utils::Colour;

use crate::Database;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::DeferredChannelMessageWithSource)
    })
    .await?;

    let data = ctx.data.read().await;
    let db = data.get::<Database>().unwrap();
    let res = sqlx::query!(
        r#"
            SELECT coins as "coins!: i64", discord_id as "discord_id!: i64"
            FROM currency
            ORDER BY coins DESC
            LIMIT 10
        "#,
    )
    .fetch_all(db)
    .await?;
    drop(data);

    let res = res
        .into_iter()
        .enumerate()
        .map(|(i, val)| format!("#{} <@{}> - {} koins", i + 1, val.discord_id, val.coins))
        .intersperse("\n".to_string())
        .collect::<String>();

    int.create_followup_message(&ctx.http, |res| {
        res.embed(|embd| {
            embd.title("Cambodia Osu Cup Koins Leaderboards")
                .description(res)
                .colour(Colour::from_rgb(0, 0, 255))
        })
    })
    .await?;

    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.name("leaderboards")
        .description("Check the leaderboards of Cambodia Osu Cup Koins (currently only shows top 10)")
}
