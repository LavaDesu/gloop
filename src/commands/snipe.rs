use std::collections::{HashMap, VecDeque};

use chrono::{Duration, Utc, DateTime};
use serenity::builder::CreateApplicationCommand;
use serenity::model::prelude::interaction::InteractionResponseType;
use serenity::model::prelude::interaction::application_command::ApplicationCommandInteraction;
use serenity::model::prelude::{ChannelId, Message, MessageId, MessageUpdateEvent};
use serenity::prelude::*;
use serenity::utils::Colour;

#[derive(Clone, Debug)]
pub enum SnipeContent {
    Delete(String),
    Edit(String, String),
}

#[derive(Clone, Debug)]
pub struct SnipeData {
    pub author: (String, String),
    pub content: SnipeContent,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct SnipeEntry {
    data: Option<SnipeData>,
}

impl SnipeEntry {
    pub fn set(&mut self, data: SnipeData) {
        self.data = Some(data);
    }

    pub fn get(&self) -> Option<&SnipeData> {
        if matches!(&self.data, Some(data) if Utc::now() - data.timestamp <= Duration::seconds(30)) {
            self.data.as_ref()
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct SnipeState {
    pub cache: HashMap<ChannelId, VecDeque<Message>>,
    pub map: HashMap<ChannelId, SnipeEntry>,
}

impl TypeMapKey for SnipeState {
    type Value = SnipeState;
}

pub async fn recv_msg(ctx: Context, msg: Message) {
    data_wscope!(ctx, state = SnipeState, {
        let vec = state.cache.entry(msg.channel_id).or_default();
        if vec.len() > 50 {
            vec.pop_back();
        }
        vec.push_front(msg);
    })
}

pub async fn recv_msg_update(ctx: Context, event: MessageUpdateEvent) {
    if let Some(content) = event.content {
        data_wscope!(ctx, state = SnipeState, {
            let old = state.cache
                .get_mut(&event.channel_id)
                .and_then(|vec| vec.iter_mut().find(|m| m.id == event.id));

            if let Some(old) = old {
                let entry = state.map.entry(event.channel_id).or_default();
                entry.set(SnipeData {
                    author: (
                        format!("{}#{:04}", old.author.name, old.author.discriminator),
                        old.author.avatar_url().unwrap_or_else(|| old.author.default_avatar_url())
                    ),
                    content: SnipeContent::Edit(old.content.clone(), content.clone()),
                    timestamp: Utc::now()
                });
                old.content = content;
            }
        });
    }
}

pub async fn recv_msg_delete(ctx: Context, channel: ChannelId, msg_id: MessageId) {
    data_wscope!(ctx, state = SnipeState, {
        let msg = state.cache
            .get(&channel)
            .and_then(|vec| vec.iter().find(|m| m.id == msg_id));

        if let Some(msg) = msg {
            let entry = state.map.entry(msg.channel_id).or_default();
            entry.set(SnipeData {
                author: (
                    format!("{}#{:04}", msg.author.name, msg.author.discriminator),
                    msg.author.avatar_url().unwrap_or_else(|| msg.author.default_avatar_url())
                ),
                content: SnipeContent::Delete(msg.content.clone()),
                timestamp: Utc::now()
            });
        }
    });
}

pub async fn init_state(client: &Client) {
    let mut data = client.data.write().await;
    data.insert::<SnipeState>(SnipeState::default());
}

pub async fn run(ctx: &Context, int: &ApplicationCommandInteraction) -> anyhow::Result<()> {
    data_scope!(ctx, state = SnipeState, {
        let msg = state.map.get(&int.channel_id)
            .and_then(|e| e.get());
        if let Some(msg) = msg {
            int.create_interaction_response(ctx, |resp| {
                resp.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|data| {
                        data.embed(|embd| {
                            embd.author(|auth| {
                                auth.name(msg.author.0.clone())
                                    .icon_url(msg.author.1.clone())
                            });
                            match &msg.content {
                                SnipeContent::Edit(old, curr) => {
                                    embd.colour(Colour::ORANGE)
                                        .fields([
                                            ("Old", format!("```{}```", old), false),
                                            ("New", format!("```{}```", curr), false),
                                        ])
                                        .footer(|foot| {
                                            foot.text(format!("Message edited {} seconds ago", (Utc::now() - msg.timestamp).num_seconds()))
                                        })
                                },
                                SnipeContent::Delete(old) => {
                                    embd.colour(Colour::RED)
                                        .description(format!("```{}```", old))
                                        .footer(|foot| {
                                            foot.text(format!("Message deleted {} seconds ago", (Utc::now() - msg.timestamp).num_seconds()))
                                        })
                                }
                            }
                        })
                    })
            }).await?;
            return Ok(());
        }
    });
    intr_emsg!(int, ctx, "No message to snipe!").await?;
    Ok(())
}

pub fn register(cmnd: &mut CreateApplicationCommand) -> &mut CreateApplicationCommand {
    cmnd.name("snipe")
        .description("Snipe message edit/deletes")
}
