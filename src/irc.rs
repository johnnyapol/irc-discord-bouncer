use std::cmp::min;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::broadcast::{Receiver, Sender};

use native_tls::TlsConnector;

use crate::message;
use base64::encode;

pub struct IRCSocket<T: AsyncRead + AsyncWrite + std::marker::Unpin> {
    addr: String,
    tls: bool,
    stream: BufReader<T>,
    tx: Sender<message::BouncerMessage>,
    nick: String,
    password: String,
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
    async fn send_raw(&mut self, irc_message: &str) -> Result<(), Box<dyn std::error::Error>> {
        assert!(irc_message.as_bytes().len() <= 512);
        self.stream.write_all(irc_message.as_bytes()).await?;
        Ok(())
    }

    async fn receive_incoming_data(&mut self, line: &mut String) -> Result<usize, String> {
        // Read from the underlying stream and propogate any errors up
        match self.stream.read_line(line).await {
            Ok(size) => {
                // Drop the CRLF terminator from the end
                if size > 1 {
                    line.truncate(line.len() - 2);
                }
                Ok(size)
            }
            Err(e) => Err(format!("{}", e)),
        }
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
                    if let Some(cmd) = x {
                        // IRC messages can only have a maximum length of 512 bytes per RFC
                        // If messages exceed this, we need to split them.
                        let text_to_process = cmd.content.split("\n").collect::<Vec<&str>>().join(" ");
                        let prefix = format!("PRIVMSG {} :", cmd.channel);
                        let prefix_len = prefix.len();
                        let mut curr_len = 0;

                        // Naive split (e.g. 'A"*512-prefix+'bcd' = all As, then bcd)
                        while curr_len < text_to_process.len() {
                            // Prepare the next part of the message to be sent
                            // 512 (max amount) - 2 (\r\n) - prefix_len (PRIVMSG CHANNEL)
                            // Note: For some reason, 446 appears to be the max we can send on libera chat
                            let offset = 446 - 2 - prefix_len;
                            let buffer_to_send = format!("{}{}\r\n", prefix, &text_to_process[curr_len..min(curr_len+offset, text_to_process.len())]);
                            curr_len += offset;
                            self.send_raw(&buffer_to_send).await?;
                        }
                    }
                },
                x = self.receive_incoming_data(&mut line) => {
                    if x.is_err() {
                        bail!(x.err().unwrap())
                    }


                    let mut split = line.split(" ");
                    let split_first = match split.next() {
                        Some(split_f) => split_f,
                        None => bail!(format!("do_main_loop: Received unexpected empty string.")),
                    };

                    match split_first {
                        "PING" => {
                            match split.next() {
                                Some(pong) => self.send_raw(&format!("PONG {}\r\n", pong)).await?,
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
                                "PRIVMSG"|"NOTICE" => {
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
                                    println!("[{}] {}", addr, line);
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
        let sasl_auth = self.password.len() > 0;

        // Authenticate using SASL (only PLAIN is supported for now)
        // Protocol info based on https://ircv3.net/specs/extensions/sasl-3.1
        // TODO: handle SASL not being available, switch to nickserv as backup
        if sasl_auth {
            self.send_raw("CAP REQ :sasl\r\n").await?;
        }

        self.send_raw(&format!(
            "NICK {}\r\nUSER discord 8 *  : {}\r\n",
            self.nick, self.nick
        ))
        .await?;

        if sasl_auth {
            let sasl_plain = encode(format!(
                "{}\x00{}\x00{}\x00",
                self.nick, self.nick, self.password
            ));
            self.send_raw(&format!(
                "AUTHENTICATE PLAIN\r\nAUTHENTICATE {}\r\nCAP END\r\n",
                sasl_plain
            ))
            .await?;
        }

        for channel in channels {
            self.send_raw(&format!("JOIN {}\r\n", channel)).await?;
        }
        self.do_main_loop().await?;
        Ok(())
    }
}

pub async fn connect_to_server(
    addr: String,
    nick: String,
    password: String,
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
            password,
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
        password,
    }
    .connect(channels)
    .await;
}
