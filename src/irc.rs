use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast::{Receiver, Sender};

use crate::message;

pub struct IRCSocket {
    addr: String,
    tls: bool,
    stream: BufReader<TcpStream>,
    tx: Sender<message::BouncerMessage>,
}

async fn process_outgoing_messages(
    rx: &mut Receiver<message::BouncerMessage>,
) -> Option<message::BouncerMessage> {
    while let Ok(cmd) = rx.recv().await {
        if cmd.state == message::MessageState::OUTGOING {
            return Some(cmd);
        }
    }

    return None;
}

impl IRCSocket {
    async fn send_raw(&mut self, irc_message: String) {
        println!("{}", irc_message);
        self.stream.write_all(irc_message.as_bytes()).await;
    }

    pub async fn send_message(&self, channel: String, message: String) {}

    async fn receive_incoming_data(&mut self, line: &mut String) {
        // more here
        self.stream.read_line(line).await;
    }

    pub async fn do_main_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut line = String::new();
        let mut rx = self.tx.subscribe();

        loop {
            tokio::select! {
                x = process_outgoing_messages(&mut rx) => {
                    match x {
                        Some(cmd) => self.send_raw(format!("PRIVMSG {} :{}\r\n", cmd.channel, cmd.content)).await,
                        None => {}
                    }
                },
                _ = self.receive_incoming_data(&mut line) => {
                    let mut split = line.split(" ");
                    let split_first = match split.next() {
                        Some(split_f) => split_f,
                        None => bail!(format!("do_main_loop: Received unexpected empty string.")),
                    };

                    let get_username_from_blob =
                        |blob: &str| -> Result<String, Box<dyn std::error::Error>> {
                            let mut chars = match blob.split("!").next() {
                                Some(chars) => chars.chars(),
                                None => bail!(format!(
                                    "get_user_name_from_blob: Unable to parse username from {}",
                                    blob
                                )),
                            };

                            chars.next();
                            return Ok(String::from(chars.as_str()));
                        };

                    match split_first {
                        "PING" => {
                            println!("Received PING message, sending PONG");

                            match split.next() {
                                Some(pong) => self.send_raw(format!("PONG {}", pong)).await,
                                None => bail!(format!(
                                    "do_main_loop: Received invalid PING message from server {}",
                                    line
                                )),
                            };
                        }
                        _ => {
                            let next_split = match split.next() {
                                Some(next_split) => next_split,
                                None => continue,
                            };

                            match next_split {
                                "PRIVMSG" => {
                                    self.tx.send(message::BouncerMessage{
                                        channel: split.next().unwrap().to_string(),
                                        network: String::from(&self.addr),
                                        state: message::MessageState::INCOMING,
                                        user: get_username_from_blob(split_first)?,
                                        content: split.collect::<Vec<&str>>().join(" ")
                                    })?;
                                }
                                "TOPIC" => {
                                    self.tx.send(message::BouncerMessage{
                                        channel: split.next().unwrap().to_string(),
                                        network: String::from(&self.addr),
                                        state: message::MessageState::INCOMING,
                                        user: get_username_from_blob(split_first)?,
                                        content:split.collect::<Vec<&str>>().join(" ")
                                    })?;
                                },
                                _ => {
                                    println!("{}", line);
                                }
                            }
                        }
                    }
                    line.clear();
                }
            };
        }
    }
}

pub async fn connect_to_server(
    addr: String,
    nick: String,
    channels: Vec<String>,
    use_tls: bool,
    tx: Sender<message::BouncerMessage>,
) -> Result<IRCSocket, Box<dyn std::error::Error>> {
    if use_tls {
        bail!("TLS is not currently supported!");
    }

    let socket_connection = TcpStream::connect(&addr).await?;
    let mut reader = BufReader::new(socket_connection);

    reader
        .write_all(
            format!(
                "NICK {}\r\nUSER irc-discord-bouncer 8 *  : {}\r\n",
                nick, nick
            )
            .as_bytes(),
        )
        .await
        .unwrap();

    for channel in channels {
        reader
            .write_all(format!("JOIN {}\r\n", channel).as_bytes())
            .await
            .unwrap();
    }

    Ok(IRCSocket {
        addr,
        tls: use_tls,
        stream: reader,
        tx,
    })
}
