use anyhow::Result;
use kdeconnect_dbus_client::KdeConnectClient;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::models::{Conversation, Message};

lazy_static::lazy_static! {
    static ref SMS_CLIENT: Arc<Mutex<Option<Arc<KdeConnectClient>>>> = Arc::new(Mutex::new(None));
}

pub async fn initialize() -> Result<()> {
    debug!("SMS D-Bus initialize()");
    let client = KdeConnectClient::new().await?;
    *SMS_CLIENT.lock().await = Some(Arc::new(client));
    info!("SMS D-Bus client initialized");
    Ok(())
}

/// Wait up to 10s for the client to be ready, then return it.
pub async fn get_client() -> Option<Arc<KdeConnectClient>> {
    for i in 0..100 {
        {
            let guard = SMS_CLIENT.lock().await;
            if let Some(c) = guard.as_ref() {
                debug!("SMS D-Bus client ready after {}*100ms", i);
                return Some(c.clone());
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
    warn!("SMS D-Bus client initialization timed out");
    None
}

pub async fn fetch_conversations(device_id: &str) {
    debug!("fetch_conversations device={}", device_id);
    let Some(client) = get_client().await else {
        return;
    };
    match client.request_conversations(device_id).await {
        Ok(_) => debug!("request_conversations sent"),
        Err(e) => error!("request_conversations failed: {:?}", e),
    }
}

pub async fn request_conversation_messages(device_id: &str, thread_id: &str) {
    debug!("request_conversation device={} thread={}", device_id, thread_id);
    let Some(client) = get_client().await else {
        return;
    };
    let tid = thread_id.parse::<i64>().unwrap_or(0);
    match client.request_conversation(device_id, tid).await {
        Ok(_) => debug!("request_conversation sent"),
        Err(e) => error!("request_conversation failed: {:?}", e),
    }
}

pub async fn send_sms(device_id: &str, phone_number: &str, message: &str) {
    debug!("send_sms to={} device={}", phone_number, device_id);
    let Some(client) = get_client().await else {
        return;
    };
    match client.send_sms(device_id, phone_number, message).await {
        Ok(_) => debug!("send_sms OK"),
        Err(e) => error!("send_sms failed: {:?}", e),
    }
}

pub async fn fetch_contacts(device_id: &str) {
    debug!("fetch_contacts device={}", device_id);
    let Some(client) = get_client().await else {
        return;
    };
    match client.request_contacts(device_id).await {
        Ok(_) => debug!("request_contacts sent"),
        Err(e) => error!("request_contacts failed: {:?}", e),
    }
}

pub async fn get_cached_contacts(device_id: &str) -> std::collections::HashMap<String, String> {
    debug!("get_cached_contacts device={}", device_id);
    let Some(client) = get_client().await else {
        return std::collections::HashMap::new();
    };
    match client.get_cached_contacts(device_id).await {
        Ok(contacts) => {
            debug!("got {} cached contacts", contacts.len());
            contacts
        }
        Err(e) => {
            error!("get_cached_contacts failed: {:?}", e);
            std::collections::HashMap::new()
        }
    }
}

pub async fn get_cached_sms(device_id: &str) -> Option<String> {
    debug!("get_cached_sms device={}", device_id);
    let Some(client) = get_client().await else {
        return None;
    };
    match client.get_cached_sms(device_id).await {
        Ok(json) if !json.is_empty() => {
            debug!("got cached SMS ({} bytes)", json.len());
            Some(json)
        }
        Ok(_) => {
            debug!("no SMS cache found");
            None
        }
        Err(e) => {
            error!("get_cached_sms failed: {:?}", e);
            None
        }
    }
}

pub fn parse_sms_messages(messages_json: &str) -> (Vec<Message>, Vec<Conversation>) {
    use std::collections::HashMap;

    let sms_data =
        match serde_json::from_str::<kdeconnect_core::plugins::sms::SmsMessages>(messages_json) {
            Ok(d) => d,
            Err(e) => {
                error!("SMS JSON parse failed: {:?}", e);
                return (vec![], vec![]);
            }
        };

    debug!("parsed {} SMS messages", sms_data.messages.len());

    let messages: Vec<Message> = sms_data
        .messages
        .iter()
        .map(|msg| {
            let address = msg
                .addresses
                .first()
                .map(|a| a.address.clone())
                .unwrap_or_default();
            Message {
                id: msg.id.to_string(),
                thread_id: msg.thread_id.to_string(),
                address,
                body: msg.body.clone(),
                date: msg.date,
                type_: msg.message_type,
                read: msg.read == 1,
            }
        })
        .collect();

    let mut groups: HashMap<String, Vec<&Message>> = HashMap::new();
    for msg in &messages {
        groups.entry(msg.thread_id.clone()).or_default().push(msg);
    }

    let conversations: Vec<Conversation> = groups
        .into_iter()
        .map(|(thread_id, mut msgs)| {
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
        })
        .collect();

    (messages, conversations)
}
