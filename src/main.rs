#[macro_use]
extern crate simple_error;
mod irc;

#[tokio::main]
async fn main() {
    println!("Hello, world!");
    let mut irc = irc::connect_to_server("irc.libera.chat:6667".to_string(), false)
        .await
        .unwrap();
    println!("Connected to chat!");
    irc.do_main_loop().await;
}
