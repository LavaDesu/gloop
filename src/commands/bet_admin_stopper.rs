use serenity::builder::CreateApplicationCommand;
use serenity::client::Context;
use serenity::model::prelude::command::CommandType;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::Permissions;

use crate::commands::bet::CtxState;
use crate::Database;

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    let data = ctx.data.read().await;

    if let Some(state) = data.get::<CtxState>() {
        if matches!(int.data.target_id, Some(id) if id.as_u64() == state.msg.0.as_u64()) {
            if let Some(stopper) = state.stopper.lock().await.take() {
                stopper.send(()).unwrap();
                data_scope!(ctx, db = Database, {
                    let mid: i64 = state.msg.0.into();
                    let datetime = chrono::offset::Utc::now();
                    sqlx::query!(
                        r#"
                            UPDATE bets
                            SET stop_time = $1
                            WHERE msg_id = $2
                        "#,
                        datetime,
                        mid
                    )
                    .execute(db)
                    .await?;
                });
                intr_emsg!(int, ctx, "Bets stopped!").await?;
                return Ok(());
            }
        }
    }

    intr_emsg!(int, ctx, "This message isn't a current, running, unstopped bet").await?;
    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.kind(CommandType::Message)
        .default_member_permissions(Permissions::MANAGE_GUILD)
        .name("Stop accepting bets")
}
