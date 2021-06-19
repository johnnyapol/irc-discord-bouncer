use serenity::{
    async_trait,
    model::{
        channel::Message,
        id::{ChannelId, GuildId, UserId},
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

use serenity::utils::Parse;
use tokio::sync::broadcast::Sender;

use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

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
    pub tls: bool,
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
    discord_user_id: UserId,
}

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Don't forward messages from non-owner
        if msg.author.id != self.discord_user_id {
            return;
        }

        println!("Channel id: {}", &msg.channel_id);

        // TODO: need to account for 512 max msg length (510 + \r\n)
        // with images and stuff this becomes a big deal;
        let mut content = String::new();

        // Check to see if the messae is a reply
        // If so, append <name>: to ping them
        match &msg.message_reference {
            Some(msg2) => {
                let msg_ref =
                    Message::parse(&ctx, &msg, &msg2.message_id.unwrap().as_u64().to_string())
                        .await
                        .unwrap();
                content.push_str(&format!("{}:", msg_ref.author.name));
            }
            None => {}
        }

        content.push_str(&msg.content);

        // append any file attachments to allow for things like image uploads, etc. to be sent
        let attachments = msg.attachments;

        for attachment in attachments {
            content.push_str(&format!(" {}", attachment.url));
        }

        if let Some(irc) = self.discord_irc_map.get(&msg.channel_id) {
            // TODO: If this method returns an Err, this means we have lost all IRC connections
            // Need to notify user of this
            // Also, the message synchronization should be changed to be 1-1 channels, that way we know specifically what failed
            self.irc_tx
                .send(message::BouncerMessage {
                    channel: String::from(&irc.channel),
                    network: String::from(&irc.addr),
                    user: "".to_string(),
                    content: content,
                    state: message::MessageState::OUTGOING,
                    ping: false,
                })
                .unwrap();
        }
    }

    async fn cache_ready(&self, ctx: Context, _guilds: Vec<GuildId>) {
        if !self.is_loop_running.load(Ordering::Relaxed) {
            self.is_loop_running.swap(true, Ordering::Relaxed);

            let ctx = ctx.clone();
            let mut rx = self.irc_tx.subscribe();

            let irc_discord_map = self.irc_discord_map.clone();
            let owner_id = self.discord_user_id.clone();

            tokio::spawn(async move {
                while let Ok(cmd) = rx.recv().await {
                    if cmd.state == message::MessageState::INCOMING {
                        if let Some(discord) = irc_discord_map.get(&IRCServer {
                            addr: cmd.network,
                            channel: cmd.channel,
                        }) {
                            let id = discord.webhook_id;
                            let token = &discord.webhook_token;
                            let user = cmd.user;
                            let webhook =
                                ctx.http.get_webhook_with_token(id, &token).await.unwrap();

                            let mut content = cmd.content;

                            lazy_static! {
                                static ref ACTION_RE: Regex =
                                    Regex::new("\x01ACTION([^\x01]+)\x01").unwrap();
                            }

                            // Handle IRC actions (e.g. /me)

                            match ACTION_RE.captures(&content) {
                                Some(caps) => {
                                    content = format!("*{}*", caps.get(1).unwrap().as_str().trim())
                                }
                                None => {}
                            }

                            let content = match cmd.ping {
                                true => format!("<@{}> {}", owner_id, content),
                                false => content,
                            };

                            let mut transmission_attempts = 0;

                            while transmission_attempts < 3 {
                                match webhook
                                    .execute(&ctx.http, false, |w| {
                                        w.content(&content)
                                            .username(&user)
                                            .avatar_url("https://i.imgur.com/4amDEwM.jpg")
                                    })
                                    .await
                                {
                                    Ok(_) => {
                                        break;
                                    }
                                    Err(_) => {
                                        transmission_attempts += 1;
                                        sleep(Duration::from_millis(100)).await;
                                    }
                                }
                            }

                            // TODO: Should we re-transmit this message?
                            if transmission_attempts == 3 {
                                println!("ERROR: Failed to send webhook {}", content);
                            }
                        } else {
                            {
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
    discord_user_id: u64,
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
            discord_user_id: UserId::from(discord_user_id),
        })
        .framework(framework)
        .await
        .expect("Error creating discord bot instance");

    if let Err(why) = client.start().await {
        println!("An error occurred while running the client: {:?}", why);
        return;
    }
}
