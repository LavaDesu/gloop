use std::time::Duration;

use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::collector::CollectModalInteraction;
use serenity::model::prelude::command::CommandType;
use serenity::model::prelude::component::{ActionRowComponent, InputTextStyle};
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::Permissions;

use crate::commands::bet::CtxState;

use super::bet::Outcome;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    let data = ctx.data.read().await;

    if let Some(state) = data.get::<CtxState>() {
        if matches!(int.data.target_id, Some(id) if id.as_u64() == state.msg.0.as_u64())
        {
            let cid = format!("outcome{}", int.id);
            let clone = cid.clone();
            int.create_interaction_response(&ctx, |resp| {
                resp.kind(InteractionResponseType::Modal)
                    .interaction_response_data(|data| {
                        data.custom_id(clone)
                            .title("Select winner")
                            .components(|cmp| {
                                cmp.create_action_row(|row| {
                                    row.create_input_text(|text| {
                                        text.custom_id("outcome")
                                            .label("Winner")
                                            .placeholder("0 for Cancelled, 1 for Red, 2 for Blue, 3 for Draw")
                                            .style(InputTextStyle::Short)
                                    })
                                })
                            })
                    })
            })
            .await?;

            let nint = CollectModalInteraction::new(&ctx.shard)
                .timeout(Duration::from_secs(300))
                .author_id(int.user.id)
                .filter(move |c| c.data.custom_id == cid)
                .await;

            if let Some(nint) = nint {
                // TODO: probably better way to do this?
                let outcome = &nint.data.components[0].components[0];
                let outcome = match outcome {
                    ActionRowComponent::InputText(e) => match e.value.as_str() {
                        "0" => Some(Outcome::Cancelled),
                        "1" => Some(Outcome::Red),
                        "2" => Some(Outcome::Blue),
                        "3" => Some(Outcome::Draw),
                        _ => None,
                    },
                    _ => None,
                };

                if let Some(outcome) = outcome {
                    if let Some(ender) = state.ender.lock().await.take() {
                        ender.send(outcome).unwrap();
                        intr_emsg!(nint, ctx, "Bets ended!").await?;
                        return Ok(());
                    }
                }

                intr_emsg!(nint, ctx, "Invalid team input or bet ended during input").await?;
            }
            return Ok(());
        }
    }

    intr_emsg!(int, ctx, "This message isn't a current, running, unended bet").await?;
    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.kind(CommandType::Message)
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .name("End and finalise bets")
}
