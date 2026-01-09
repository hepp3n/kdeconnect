use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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
    pub actions: Option<Vec<String>>,
    #[serde(rename = "payloadHash")]
    pub payload_hash: Option<String>,
}

#[allow(dead_code)]
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NotificationAction {
    pub key: Option<String>,
    pub action: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[allow(dead_code)]
pub struct NotificationReply {
    #[serde(rename = "requestReplyId")]
    pub request_reply_id: Option<String>,
    pub message: Option<String>,
}

#[allow(dead_code)]
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
        device: &crate::device::Device,
        _core_event: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        // Implementation for handling received notifications would go here.
        let _device_id = device.device_id.clone();
        let app_name = self.app_name.clone().unwrap_or_default();
        let Some(title) = self.title.clone() else {
            return;
        };
        let Some(text) = self.text.clone() else {
            return;
        };
        let _key = self.id.clone().unwrap_or_default();

        let actions = self.actions.clone().unwrap_or_default();

        let _ = tokio::task::spawn_blocking(move || {
            // let mut notify_action: Option<String> = None;

            let mut notify = notify_rust::Notification::new();
            notify.appname(&app_name);
            notify.summary(&title);
            notify.body(&text);

            for action in actions.iter() {
                notify.action(action, action);
            }

            notify
                // .action("clicked", "Click to reply")
                .hint(notify_rust::Hint::Resident(true))
                .show()
                .unwrap();
            // .wait_for_action(|action| {
            //     notify_action = Some(action.to_string());
            // });

            // if let Some(action) = notify_action {
            //     let packet = crate::protocol::ProtocolPacket::new(
            //         crate::protocol::PacketType::NotificationAction,
            //         serde_json::to_value(NotificationAction {
            //             key: Some(key),
            //             action: Some(action),
            //         })
            //         .unwrap_or_default(),
            //     );
            //
            //     core_event
            //         .send(crate::event::CoreEvent::SendPacket {
            //             device: device_id,
            //             packet,
            //         })
            //         .unwrap_or_default();
            // }
        })
        .await;
    }
}
