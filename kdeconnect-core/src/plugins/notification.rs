use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    process::{Command, Stdio},
    sync::Mutex,
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{
    device::Device,
    event::CoreEvent,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

static NOTIFICATION_IDS: Lazy<Mutex<HashMap<String, u32>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Notification {
    pub id: Option<String>,
    pub title: Option<String>,
    pub text: Option<String>,
    pub ticker: Option<String>,
    #[serde(rename = "appName")]
    pub app_name: Option<String>,
    #[serde(rename = "isClearable")]
    pub is_clearable: Option<bool>,
    pub silent: Option<bool>,
    #[serde(rename = "requestReplyId")]
    pub request_reply_id: Option<String>,
    pub time: Option<String>,
    pub actions: Option<Vec<String>>,
    #[serde(rename = "payloadHash")]
    pub payload_hash: Option<String>,
    #[serde(rename = "isCancel")]
    pub is_cancel: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationAction {
    pub key: Option<String>,
    pub action: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationReply {
    #[serde(rename = "requestReplyId")]
    pub request_reply_id: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationRequest {
    pub cancel: Option<String>,
    pub request: Option<bool>,
}

impl Plugin for Notification {
    fn id(&self) -> &'static str {
        "kdeconnect.notification"
    }
}

impl Notification {
    pub async fn received_packet(
        &self,
        device: &Device,
        core_event: mpsc::UnboundedSender<CoreEvent>,
    ) {
        let key = self.id.clone().unwrap_or_default();

        if self.is_cancel.unwrap_or(false) {
            close_notification(&key).await;
            return;
        }

        let device_id = device.device_id.clone();
        let app_name = self.app_name.clone().unwrap_or_default();
        let Some(title) = self.title.clone() else {
            return;
        };
        let text = self.text.clone().unwrap_or_default();
        let actions = self.actions.clone().unwrap_or_default();
        let reply_id = self.request_reply_id.clone();
        let original = self.clone();

        let _ = tokio::task::spawn_blocking(move || {
            let mut notify = notify_rust::Notification::new();
            notify.appname(&app_name);
            notify.summary(if app_name.is_empty() {
                &title
            } else {
                &app_name
            });
            notify.body(&notification_body(&title, &text, &app_name));

            for action in actions.iter() {
                notify.action(action, action);
            }

            if reply_id.is_some() {
                notify.action("reply", "Reply");
            }

            let handle = match notify.hint(notify_rust::Hint::Resident(true)).show() {
                Ok(handle) => handle,
                Err(e) => {
                    warn!("[notification] failed to show notification: {}", e);
                    return;
                }
            };

            if !key.is_empty() {
                NOTIFICATION_IDS
                    .lock()
                    .unwrap()
                    .insert(key.clone(), handle.id());
            }

            handle.wait_for_action(|action| {
                if !key.is_empty() {
                    NOTIFICATION_IDS.lock().unwrap().remove(&key);
                }

                match action {
                    "__closed" => {
                        if !key.is_empty() {
                            let packet = ProtocolPacket::new(
                                PacketType::NotificationRequest,
                                serde_json::json!({ "cancel": key }),
                            );
                            let _ = core_event.send(CoreEvent::SendPacket {
                                device: device_id,
                                packet,
                            });
                        }
                    }
                    "reply" => {
                        if let Some(request_reply_id) = reply_id
                            && let Some(message) = prompt_reply(&original)
                        {
                            let packet = ProtocolPacket::new(
                                PacketType::NotificationReply,
                                serde_json::to_value(NotificationReply {
                                    request_reply_id: Some(request_reply_id),
                                    message: Some(message),
                                })
                                .unwrap_or_default(),
                            );
                            let _ = core_event.send(CoreEvent::SendPacket {
                                device: device_id,
                                packet,
                            });
                        }
                    }
                    selected => {
                        let packet = ProtocolPacket::new(
                            PacketType::NotificationAction,
                            serde_json::to_value(NotificationAction {
                                key: Some(key.clone()),
                                action: Some(selected.to_string()),
                            })
                            .unwrap_or_default(),
                        );
                        let _ = core_event.send(CoreEvent::SendPacket {
                            device: device_id,
                            packet,
                        });
                    }
                }
            });
        })
        .await;
    }
}

impl NotificationRequest {
    pub async fn received_packet(&self) {
        if let Some(cancel) = self.cancel.as_deref() {
            close_notification(cancel).await;
        }

        if self.request.unwrap_or(false) {
            debug!(
                "[notification] phone requested local notification list; passive desktop notification mirroring is not available yet"
            );
        }
    }
}

fn notification_body(title: &str, text: &str, app_name: &str) -> String {
    if app_name.is_empty() || app_name == title {
        return text.to_string();
    }

    if text.is_empty() {
        title.to_string()
    } else {
        format!("{}: {}", title, text)
    }
}

fn prompt_reply(notification: &Notification) -> Option<String> {
    let title = notification
        .title
        .as_deref()
        .or(notification.app_name.as_deref())
        .unwrap_or("Reply");
    let text = notification.text.as_deref().unwrap_or_default();

    let output = Command::new("zenity")
        .arg("--entry")
        .arg("--title")
        .arg(format!("Reply to {}", title))
        .arg("--text")
        .arg(text)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let message = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if message.is_empty() {
        None
    } else {
        Some(message)
    }
}

async fn close_notification(key: &str) {
    let Some(id) = NOTIFICATION_IDS.lock().unwrap().remove(key) else {
        return;
    };

    let connection = match zbus::Connection::session().await {
        Ok(connection) => connection,
        Err(e) => {
            warn!("[notification] failed to connect to session bus: {}", e);
            return;
        }
    };

    if let Err(e) = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &(id),
        )
        .await
    {
        warn!("[notification] failed to close notification {}: {}", id, e);
    }
}

#[cfg(test)]
mod tests {
    use super::notification_body;

    #[test]
    fn formats_remote_notification_body_like_gsconnect() {
        assert_eq!(
            notification_body("Message", "Hello", "Messages"),
            "Message: Hello"
        );
        assert_eq!(notification_body("Messages", "Hello", "Messages"), "Hello");
    }
}
