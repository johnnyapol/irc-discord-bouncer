use std::fmt::{Display, Formatter};

#[derive(Debug, PartialEq)]
pub enum MessageState {
    INCOMING,
    OUTGOING,
}

impl Copy for MessageState {}
impl Clone for MessageState {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Debug)]
pub struct BouncerMessage {
    pub network: String,
    pub channel: String,
    pub user: String,
    pub content: String,
    pub state: MessageState,
}

impl Clone for BouncerMessage {
    fn clone(&self) -> Self {
        BouncerMessage {
            network: self.network.clone(),
            channel: self.channel.clone(),
            user: self.user.clone(),
            content: self.content.clone(),
            state: self.state,
        }
    }
}

impl Display for BouncerMessage {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(
            formatter,
            "BouncerMessage({}, {}, {}, {}, {:?})",
            self.network, self.channel, self.user, self.content, self.state
        )?;
        Ok(())
    }
}
