use std::time::Duration;

use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::collector::CollectModalInteraction;
use serenity::model::Permissions;
use serenity::model::prelude::command::CommandType;
use serenity::model::prelude::component::{ActionRowComponent, InputTextStyle};
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;

use crate::commands::bet::CtxState;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    let data = ctx.data.read().await;

    if let Some(state) = data.get::<CtxState>() {
        if matches!(int.data.target_id, Some(id) if id.as_u64() == state.read().await.msg.0.as_u64()) {
            let cid = format!("winner{}", int.id);
            let clone = cid.clone();
            int.create_interaction_response(&ctx, |resp| {
                resp.kind(InteractionResponseType::Modal)
                    .interaction_response_data(|data| {
                        data.custom_id(clone)
                            .title("Select winner")
                            .components(|c| c.create_action_row(|roww| {
                                roww.create_input_text(|text| {
                                    text.custom_id("winner")
                                        .label("Winner")
                                        .placeholder("1 for Red (1st), 2 for Blue (2nd)")
                                        .style(InputTextStyle::Short)
                                })
                            }))
                    })
            }).await?;

            let nint = CollectModalInteraction::new(&ctx.shard)
                .timeout(Duration::from_secs(300))
                .author_id(int.user.id)
                .filter(move |c| c.data.custom_id == cid)
                .await;

            if let Some(nint) = nint {
                // TODO: probably better way to do this?
                let winner = &nint.data.components[0].components[0];
                let winner = match winner {
                    ActionRowComponent::InputText(e) => match e.value.as_str() {
                        "1" => Some(false),
                        "2" => Some(true),
                        _ => None,
                    }
                    _ => None
                };

                if let Some(win) = winner {
                    let mut state = state.write().await;
                    if let Some(ender) = state.ender.take() {
                        ender.send(win).unwrap();
                        intr_emsg!(nint, ctx, "Bets ended!").await?;
                        return Ok(());
                    }
                }

                intr_emsg!(nint, ctx, "Invalid team input or bet ended during input").await?;
                return Ok(());
            }
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
