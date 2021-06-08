use serenity::{
    async_trait,
    model::{
        channel::Message,
        id::{ChannelId, GuildId},
    },
};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

use crate::message;
use serenity::client::{Client, Context, EventHandler};
use serenity::framework::standard::{
    macros::{command, group},
    CommandResult, StandardFramework,
};
use tokio::sync::broadcast::Sender;

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(PartialEq, Eq, Hash)]
struct IRCServer {
    addr: String,
    channel: String,
}

#[derive(Serialize, Deserialize)]
pub struct IRCChannel {
    pub name: String,
    discord_channel: u64,
    webhook_url: String,
}

#[derive(Serialize, Deserialize)]
pub struct IRCServerConfig {
    pub address: String,
    pub nick: String,
    pub channels: Vec<IRCChannel>,
}

struct DiscordChannel {
    webhook_id: u64,
    webhook_token: String,
}

#[group]
struct General;
struct Handler {
    is_loop_running: AtomicBool,
    irc_tx: Sender<message::BouncerMessage>,
    discord_irc_map: HashMap<ChannelId, IRCServer>,
    irc_discord_map: Arc<HashMap<IRCServer, DiscordChannel>>,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, _ctx: Context, msg: Message) {
        // Don't forward bot messages (avoids infinite loop)
        if msg.author.bot {
            return;
        }

        println!("Channel id: {}", &msg.channel_id);

        match self.discord_irc_map.get(&msg.channel_id) {
            Some(irc) => {
                self.irc_tx.send(message::BouncerMessage {
                    channel: String::from(&irc.channel),
                    network: String::from(&irc.addr),
                    user: "".to_string(),
                    content: msg.content,
                    state: message::MessageState::OUTGOING,
                });
            }
            None => {}
        }
    }

    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        if !self.is_loop_running.load(Ordering::Relaxed) {
            self.is_loop_running.swap(true, Ordering::Relaxed);

            let ctx = ctx.clone();
            let mut rx = self.irc_tx.subscribe();

            let irc_discord_map = self.irc_discord_map.clone();

            tokio::spawn(async move {
                while let Ok(cmd) = rx.recv().await {
                    if cmd.state == message::MessageState::INCOMING {
                        match irc_discord_map.get(&IRCServer {
                            addr: cmd.network,
                            channel: cmd.channel,
                        }) {
                            Some(discord) => {
                                let id = discord.webhook_id;
                                let token = &discord.webhook_token;
                                let content = cmd.content;
                                let user = cmd.user;
                                let webhook =
                                    ctx.http.get_webhook_with_token(id, &token).await.unwrap();
                                webhook
                                    .execute(&ctx.http, false, |w| {
                                        w.content(content).username(user)
                                    })
                                    .await;
                            }
                            None => {
                                println!("ERROR: Received message from unexpected channel.");
                            }
                        }
                    }
                }
            });
        }
    }
}

fn webhook_from_url(webhook_url: &str) -> Option<DiscordChannel> {
    lazy_static! {
        static ref RE: Regex = Regex::new(r".+/webhooks/([^/]+)/([^/]+)").unwrap();
    }
    let captures = RE.captures(webhook_url).unwrap();
    Some(DiscordChannel {
        webhook_id: captures
            .get(1)
            .map_or(0, |m| m.as_str().parse::<u64>().unwrap()),
        webhook_token: captures
            .get(2)
            .map_or("".to_string(), |m| m.as_str().to_string()),
    })
}

pub async fn discord_init(
    token: &str,
    servers: Vec<IRCServerConfig>,
    tx: Sender<message::BouncerMessage>,
) {
    let framework = StandardFramework::new()
        .configure(|c| c.prefix("/"))
        .group(&GENERAL_GROUP);

    let mut irc_discord_map = HashMap::new();
    let mut discord_irc_map = HashMap::new();

    for server in servers {
        for channel in server.channels {
            irc_discord_map.insert(
                IRCServer {
                    addr: String::from(&server.address),
                    channel: String::from(&channel.name),
                },
                webhook_from_url(&channel.webhook_url).unwrap(),
            );

            discord_irc_map.insert(
                ChannelId(channel.discord_channel),
                IRCServer {
                    addr: String::from(&server.address),
                    channel: String::from(&channel.name),
                },
            );
        }
    }

    let mut client = Client::builder(token)
        .event_handler(Handler {
            is_loop_running: AtomicBool::new(false),
            irc_tx: tx,
            discord_irc_map: discord_irc_map,
            irc_discord_map: Arc::new(irc_discord_map),
        })
        .framework(framework)
        .await
        .expect("Error creating discord bot instance");

    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
        return;
    }
}
