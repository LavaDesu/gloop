use std::time::Duration;

use serenity::builder::{CreateApplicationCommand, CreateComponents};
use serenity::futures::StreamExt;
use serenity::model::application::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::channel::Message;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::prelude::*;

async fn cleanup(ctx: &Context, msg: &mut Message) -> Result<(), SerenityError> {
    msg.edit(&ctx.http, |m| m.set_components(CreateComponents::default()).content("i no longer have buttons")).await
}

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    let mut msg = int.channel_id.send_message(&ctx.http, |m| {
        m.content("i have buttons").components(|c| {
            c.create_action_row(|roww| {
                roww.create_button(|butn| butn.custom_id("btnone").label("i am button 1"))
                    .create_button(|butn| butn.custom_id("btntwo").label("i am button 2"))
            })
        })
    }).await?;

    int.create_interaction_response(&ctx.http, |resp| {
        resp.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|m| m.ephemeral(true).content("sentt"))
    }).await?;

    let mut interaction_stream = msg.await_component_interactions(ctx).timeout(Duration::from_secs(5)).build();

    while let Some(interaction) = interaction_stream.next().await {
        let button = match interaction.data.custom_id.as_str() {
            "btnone" => "1",
            "btntwo" => "2",
            _ => "what the hell kind of button did you press"
        };
        interaction
            .create_interaction_response(&ctx, |resp| {
                resp.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| {
                        data.ephemeral(true)
                            .content("you have pressed button ".to_owned() + button)
                    })
            })
            .await?;
    }

    cleanup(ctx, &mut msg).await?;

    Ok(())
}

pub fn register(command: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    command.name("buttontest").description("buttons, very wow")
}

/*pub async fn _run(ctx: &Context, cmd: &ApplicationCommandInteraction, interaction: &Interaction) -> anyhow::Result<(), SerenityError> {
    let m = cmd.create_interaction_response(&ctx.http, |data| {
        data.kind(InteractionResponseType::ChannelMessageWithSource)
            .interaction_response_data(|msg| {
                msg.content("pong").components(|c| {
                    c.create_action_row(|row| {
                        /*row.create_select_menu(|menu| {
                            menu.custom_id("pingmenu")
                                .placeholder(label)
                        })*/
                        row.create_button(|butn| {
                            butn.custom_id("pingbutn")
                                .style(serenity::model::prelude::component::ButtonStyle::Primary)
                                .label("i am a button")
                        })
                    })
                })
            })
    })
    .await?;

    let interaction =
        match cmd.await_component_interaction(&ctx).timeout(Duration::from_secs(60 * 3)).await {
            Some(x) => x,
            None => {
                m.reply(&ctx, "Timed out").await.unwrap();
                return;
            },
        };

    Ok(())
}*/
