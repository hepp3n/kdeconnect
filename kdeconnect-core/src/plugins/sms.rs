use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::info;

use crate::event::ConnectionEvent;
use crate::plugin_interface::Plugin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessages {
    pub messages: Vec<SmsMessage>,
    pub version: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsMessage {
    #[serde(rename = "_id")]
    pub id: i64,
    pub addresses: Vec<SmsAddress>,
    #[serde(default)]
    pub attachments: Vec<SmsAttachment>,
    pub body: String,
    pub date: i64,
    #[serde(rename = "type")]
    pub message_type: i32,
    pub read: i32,
    pub thread_id: i64,
    #[serde(default)]
    pub sub_id: Option<i32>,
    #[serde(default)]
    pub event: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsAddress {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmsAttachment {
    pub part_id: i64,
    pub mime_type: String,
    pub encoded_thumbnail: Option<String>,
    pub unique_identifier: Option<String>,
}

impl SmsMessages {
    pub async fn received_packet(&self, tx: mpsc::UnboundedSender<ConnectionEvent>) {
        info!(
            "Received SMS messages packet with {} messages",
            self.messages.len()
        );

        let event = ConnectionEvent::SmsMessages(self.clone());
        let _ = tx.send(event);
    }
}

impl Plugin for SmsMessages {
    fn id(&self) -> &'static str {
        "kdeconnect.sms.messages"
    }
}
