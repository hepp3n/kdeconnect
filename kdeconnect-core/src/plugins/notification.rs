use serde::{Deserialize, Serialize};
use std::fmt::Display;

use crate::plugin_interface::Plugin;

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
    pub actions: Option<Vec<Action>>,
    #[serde(rename = "payloadHash")]
    pub payload_hash: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub enum Action {
    #[default]
    Ignore,
    Open,
}

impl From<&str> for Action {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "ignore" => Action::Ignore,
            "open" => Action::Open,
            _ => Action::Ignore,
        }
    }
}

impl From<String> for Action {
    fn from(s: String) -> Self {
        Action::from(s.as_str())
    }
}

impl Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Action::Ignore => "Ignore",
            Action::Open => "Open",
        };
        write!(f, "{}", s)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationAction {
    pub key: Option<String>,
    pub action: Option<Action>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationReply {
    #[serde(rename = "requestReplyId")]
    pub request_reply_id: Option<String>,
    pub message: Option<String>,
}

pub struct NotificationRequest {
    pub cancel: Option<String>,
    pub request: Option<bool>,
}

#[async_trait::async_trait]
impl Plugin for Notification {
    fn id(&self) -> &'static str {
        "kdeconnect.notification"
    }

    async fn received(
        &self,
        _device: &crate::device::Device,
        _event: std::sync::Arc<tokio::sync::mpsc::UnboundedSender<crate::event::ConnectionEvent>>,
        _core_event: std::sync::Arc<tokio::sync::broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        // Implementation for handling received notifications would go here.
        let app_name = self.app_name.clone().unwrap_or_default();
        let title = self.title.clone().unwrap_or_default();
        let text = self.text.clone().unwrap_or_default();

        let _ = tokio::task::spawn_blocking(move || {
            notify_rust::Notification::new()
                .appname(&app_name)
                .summary(&title)
                .body(&text)
                // .action("clicked", "Click to reply")
                .hint(notify_rust::Hint::Resident(true))
                .show()
                .unwrap()
                .wait_for_action(|action| match action {
                    "clicked" => {}
                    // here "__closed" is a hard coded keyword
                    "__closed" => println!("the notification was closed"),
                    _ => (),
                });
        })
        .await;
    }
    async fn send(
        &self,
        _device: &crate::device::Device,
        _core_event: std::sync::Arc<tokio::sync::broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        // Implementation for sending notifications would go here.
    }
}
