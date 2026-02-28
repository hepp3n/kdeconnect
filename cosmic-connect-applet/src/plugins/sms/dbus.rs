// cosmic-connect-applet/src/plugins/sms/dbus.rs
use anyhow::Result;
use kdeconnect_dbus_client::KdeConnectClient;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::models::{Conversation, Message};

lazy_static::lazy_static! {
    static ref SMS_CLIENT: Arc<Mutex<Option<Arc<KdeConnectClient>>>> = Arc::new(Mutex::new(None));
}

pub async fn initialize() -> Result<()> {
    eprintln!("[SMS-DBUS] initialize()");
    let client = KdeConnectClient::new().await?;
    *SMS_CLIENT.lock().await = Some(Arc::new(client));
    eprintln!("[SMS-DBUS] initialize() OK");
    Ok(())
}

/// Wait up to 10s for the client to be ready, then return it.
pub async fn get_client() -> Option<Arc<KdeConnectClient>> {
    for i in 0..100 {
        {
            let guard = SMS_CLIENT.lock().await;
            if let Some(c) = guard.as_ref() {
                eprintln!("[SMS-DBUS] get_client() ready after {}*100ms", i);
                return Some(c.clone());
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    eprintln!("[SMS-DBUS] get_client() TIMEOUT");
    None
}

pub async fn fetch_conversations(device_id: &str) {
    eprintln!("[SMS-DBUS] fetch_conversations() device={}", device_id);
    let Some(client) = get_client().await else { return; };
    match client.request_conversations(device_id).await {
        Ok(_) => eprintln!("[SMS-DBUS] request_conversations sent OK"),
        Err(e) => eprintln!("[SMS-DBUS] request_conversations FAILED: {:?}", e),
    }
}

pub async fn request_conversation_messages(device_id: &str, thread_id: &str) {
    eprintln!("[SMS-DBUS] request_conversation device={} thread={}", device_id, thread_id);
    let Some(client) = get_client().await else { return; };
    let tid = thread_id.parse::<i64>().unwrap_or(0);
    match client.request_conversation(device_id, tid).await {
        Ok(_) => eprintln!("[SMS-DBUS] request_conversation sent OK"),
        Err(e) => eprintln!("[SMS-DBUS] request_conversation FAILED: {:?}", e),
    }
}

pub async fn send_sms(device_id: &str, phone_number: &str, message: &str) {
    eprintln!("[SMS-DBUS] send_sms to={} device={}", phone_number, device_id);
    let Some(client) = get_client().await else { return; };
    match client.send_sms(device_id, phone_number, message).await {
        Ok(_) => eprintln!("[SMS-DBUS] send_sms OK"),
        Err(e) => eprintln!("[SMS-DBUS] send_sms FAILED: {:?}", e),
    }
}

pub fn parse_sms_messages(messages_json: &str) -> (Vec<Message>, Vec<Conversation>) {
    use std::collections::HashMap;

    let sms_data = match serde_json::from_str::<kdeconnect_core::plugins::sms::SmsMessages>(messages_json) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[SMS-DBUS] JSON parse FAILED: {:?}", e);
            return (vec![], vec![]);
        }
    };

    eprintln!("[SMS-DBUS] parsed {} messages", sms_data.messages.len());

    let messages: Vec<Message> = sms_data.messages.iter().map(|msg| {
        let address = msg.addresses.first().map(|a| a.address.clone()).unwrap_or_default();
        Message {
            id: msg.id.to_string(),
            thread_id: msg.thread_id.to_string(),
            address,
            body: msg.body.clone(),
            date: msg.date,
            type_: msg.message_type,
            read: msg.read == 1,
        }
    }).collect();

    let mut groups: HashMap<String, Vec<&Message>> = HashMap::new();
    for msg in &messages {
        groups.entry(msg.thread_id.clone()).or_default().push(msg);
    }

    let conversations: Vec<Conversation> = groups.into_iter().map(|(thread_id, mut msgs)| {
        msgs.sort_by(|a, b| b.date.cmp(&a.date));
        let last = msgs.first().unwrap();
        Conversation {
            thread_id,
            phone_number: last.address.clone(),
            last_message: last.body.clone(),
            timestamp: last.date,
            unread: msgs.iter().any(|m| !m.read),
            contact_name: String::new(),
        }
    }).collect();

    (messages, conversations)
}
