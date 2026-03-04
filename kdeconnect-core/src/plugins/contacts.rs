use std::collections::HashMap;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::info;

use crate::event::ConnectionEvent;

/// Parses a vCard string and returns (Option<name>, Vec<phone_numbers>)
pub fn parse_vcard(content: &str) -> (Option<String>, Vec<String>) {
    let mut name: Option<String> = None;
    let mut phones: Vec<String> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("FN:") {
            name = Some(line[3..].trim().to_string());
        } else if name.is_none() && line.starts_with("N:") {
            let parts: Vec<&str> = line[2..].split(';').collect();
            if parts.len() >= 2 {
                let full = format!("{} {}", parts[1].trim(), parts[0].trim())
                    .trim()
                    .to_string();
                if !full.is_empty() {
                    name = Some(full);
                }
            }
        } else if line.starts_with("TEL") {
            if let Some(pos) = line.rfind(':') {
                let phone = line[pos + 1..].trim().to_string();
                if !phone.is_empty() {
                    phones.push(phone);
                }
            }
        }
    }

    (name, phones)
}

/// Parses a `kdeconnect.contacts.response_uids_timestamps` packet body
/// and returns the list of UIDs.
pub fn parse_uids_timestamps(body: &Value) -> Vec<String> {
    let uids: Vec<String> = match body.get("uids").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => return vec![],
    };
    info!("[contacts] received {} UIDs from phone", uids.len());
    uids
}

/// Parses a `kdeconnect.contacts.response_vcards` packet body and emits
/// a `ConnectionEvent::ContactsReceived` with a phone → name map.
pub fn parse_vcards_and_emit(body: &Value, tx: &mpsc::UnboundedSender<ConnectionEvent>) {
    let uids: Vec<String> = match body.get("uids").and_then(|v| v.as_array()) {
        Some(arr) => arr.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => {
            tracing::warn!("[contacts] response_vcards missing 'uids' field");
            return;
        }
    };

    let mut contacts: HashMap<String, String> = HashMap::new();

    for uid in &uids {
        if let Some(vcard_str) = body.get(uid).and_then(|v| v.as_str()) {
            let (name, phones) = parse_vcard(vcard_str);
            if let Some(name) = name {
                for phone in phones {
                    contacts.insert(phone, name.clone());
                }
            }
        }
    }

    info!("[contacts] parsed {} contacts from {} vCards", contacts.len(), uids.len());
    let _ = tx.send(ConnectionEvent::ContactsReceived(contacts));
}

/// Builds the body for a `kdeconnect.contacts.request_vcards_by_uid` packet.
pub fn build_vcards_request(uids: Vec<String>) -> Value {
    serde_json::json!({ "uids": uids })
}
