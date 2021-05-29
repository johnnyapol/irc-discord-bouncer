use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct IRCSocket {
    addr: String,
    tls: bool,
    stream: BufReader<TcpStream>,
}

impl IRCSocket {
    async fn send_raw(&mut self, irc_message: String) {
        self.stream.write_all(irc_message.as_bytes()).await;
    }

    pub async fn send_message(&self, channel: String, message: String) {}

    pub async fn do_main_loop(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut line = String::new();

        loop {
            if self.stream.read_line(&mut line).await? == 0 {
                // Connection was terminated
                return Ok(());
            }

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
                            println!(
                                "[{}] {}: {}",
                                next_split,
                                get_username_from_blob(split_first)?,
                                split.collect::<Vec<&str>>().join(" ")
                            );
                        }
                        "TOPIC" => {
                            println!(
                                "Topic has been changed on {} by {} to {}",
                                next_split,
                                get_username_from_blob(split_first)?,
                                split.collect::<Vec<&str>>().join(" ")
                            );
                        }
                        _ => {
                            println!("{}", line);
                        }
                    }
                }
            }

            line.clear();
        }

        Ok(())
    }
}

pub async fn connect_to_server(
    addr: String,
    use_tls: bool,
) -> Result<IRCSocket, Box<dyn std::error::Error>> {
    if (use_tls) {
        bail!("TLS is not currently supported!");
    }

    let socket_connection = TcpStream::connect(&addr).await?;
    let mut reader = BufReader::new(socket_connection);

    reader
        .write_all(
            "NICK me_jeffrey\r\nUSER irc-discord-bouncer 8 *  : me_jeffrey\r\nJOIN ##john-test\r\n"
                .as_bytes(),
        )
        .await
        .unwrap();

    Ok(IRCSocket {
        addr,
        tls: use_tls,
        stream: reader,
    })
}
