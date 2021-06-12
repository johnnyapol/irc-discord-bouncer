use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast::{Receiver, Sender};

use native_tls::TlsConnector;

use crate::message;

pub struct IRCSocket<T: AsyncRead + AsyncWrite + std::marker::Unpin> {
    addr: String,
    tls: bool,
    stream: BufReader<T>,
    tx: Sender<message::BouncerMessage>,
    nick: String,
}

async fn process_outgoing_messages(
    rx: &mut Receiver<message::BouncerMessage>,
    addr: &str,
) -> Option<message::BouncerMessage> {
    if let Ok(cmd) = rx.recv().await {
        if cmd.state == message::MessageState::OUTGOING && cmd.network == addr {
            return Some(cmd);
        }
    }
    return None;
}

impl<T: AsyncRead + AsyncWrite + std::marker::Unpin> IRCSocket<T> {
    async fn send_raw(&mut self, irc_message: String) {
        self.stream.write_all(irc_message.as_bytes()).await;
    }

    async fn receive_incoming_data(&mut self, line: &mut String) {
        // more here
        self.stream.read_line(line).await;
    }

    pub async fn do_main_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut line = String::new();
        let mut rx = self.tx.subscribe();
        let addr = String::from(&self.addr);

        let get_username_from_blob = |blob: &str| -> Result<String, Box<dyn std::error::Error>> {
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

        loop {
            tokio::select! {
                x = process_outgoing_messages(&mut rx, &addr) => {
                    match x {
                        Some(cmd) => self.send_raw(format!("PRIVMSG {} :{}\r\n", cmd.channel, cmd.content.split("\n").collect::<Vec<&str>>().join(" "))).await,
                        None => {}
                    }
                },
                _ = self.receive_incoming_data(&mut line) => {
                    let mut split = line.split(" ");
                    let split_first = match split.next() {
                        Some(split_f) => split_f,
                        None => bail!(format!("do_main_loop: Received unexpected empty string.")),
                    };

                    match split_first {
                        "PING" => {
                            match split.next() {
                                Some(pong) => self.send_raw(format!("PONG {}\r\n", pong)).await,
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
                                    let channel = split.next().unwrap().to_string();
                                    let content =  String::from(&split.collect::<Vec<&str>>().join(" ").as_str()[1..]);
                                    let ping = content.contains(&self.nick);
                                    self.tx.send(message::BouncerMessage{
                                        channel,
                                        network: String::from(&self.addr),
                                        state: message::MessageState::INCOMING,
                                        user: get_username_from_blob(split_first)?,
                                        content,
                                        ping
                                    })?;
                                }
                                "TOPIC" => {
                                    self.tx.send(message::BouncerMessage{
                                        channel: split.next().unwrap().to_string(),
                                        network: String::from(&self.addr),
                                        state: message::MessageState::INCOMING,
                                        user: get_username_from_blob(split_first)?,
                                        content:split.collect::<Vec<&str>>().join(" "),
                                        ping: false
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

    pub async fn connect(
        &mut self,
        channels: Vec<String>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send_raw(format!(
            "NICK {}\r\nUSER irc-discord-bouncer 8 *  : {}\r\n",
            self.nick, self.nick
        ))
        .await;

        for channel in channels {
            self.send_raw(format!("JOIN {}\r\n", channel)).await;
        }
        self.do_main_loop().await?;
        Ok(())
    }
}

pub async fn connect_to_server(
    addr: String,
    nick: String,
    channels: Vec<String>,
    use_tls: bool,
    tx: Sender<message::BouncerMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket_connection = TcpStream::connect(&addr).await?;

    if !use_tls {
        // Can just short-circuit with the existing stream
        return IRCSocket {
            addr,
            tls: use_tls,
            stream: BufReader::new(socket_connection),
            tx,
            nick,
        }
        .connect(channels)
        .await;
    }

    let stream = BufReader::new(
        tokio_native_tls::TlsConnector::from(TlsConnector::builder().build()?)
            .connect(&addr.split(":").next().unwrap(), socket_connection)
            .await?,
    );

    return IRCSocket {
        addr,
        tls: use_tls,
        stream,
        tx,
        nick,
    }
    .connect(channels)
    .await;
}
