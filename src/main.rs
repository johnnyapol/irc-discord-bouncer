#[macro_use]
extern crate simple_error;
mod discord;
mod irc;
mod message;

use tokio::sync::broadcast;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Config {
    token: String,
    discord_user_id: u64,
    servers: Vec<discord::IRCServerConfig>,
}

use std::fs;

#[tokio::main]
async fn main() {
    let data: Config =
        serde_json::from_str(&fs::read_to_string("config.json").expect("Failed to load config"))
            .unwrap();
    let (tx, mut rx) = broadcast::channel(32);

    for server in &data.servers {
        tokio::spawn({
            let tx_irc = tx.clone();
            let server_addr = String::from(&server.address);
            let nick = String::from(&server.nick);
            let channels = &server.channels;
            let use_tls = server.tls;
            let password = match &server.password {
                Some(passwd) => String::from(passwd),
                None => String::new(),
            };

            let mut chans: Vec<String> = Vec::new();

            for chan in channels {
                chans.push(String::from(&chan.name));
            }

            async move {
                irc::connect_to_server(server_addr, nick, password, chans, use_tls, tx_irc)
                    .await
                    .unwrap();
            }
        });
    }

    let tx_discord = tx.clone();

    tokio::spawn(async move {
        while let Ok(cmd) = rx.recv().await {
            println!("{}", cmd);
        }
    });

    discord::discord_init(&data.token, data.discord_user_id, data.servers, tx_discord).await;
}
