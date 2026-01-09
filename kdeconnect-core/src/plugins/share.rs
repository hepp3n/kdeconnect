use std::{path::PathBuf, str::FromStr};

use ashpd::desktop::file_chooser::SelectedFiles;
use notify_rust::Timeout;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
    device::Device,
    plugin_interface::Plugin,
    protocol::{PacketPayloadTransferInfo, PacketType, ProtocolPacket},
    transport::receive_payload,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum ShareRequest {
    File(ShareRequestFile),
    Text { text: String },
    Url { url: String },
}

impl Default for ShareRequest {
    fn default() -> Self {
        ShareRequest::Text {
            text: "Hello From COSMIC Desktop".to_string(),
        }
    }
}

impl ShareRequest {
    pub async fn share_files(content: Vec<String>) -> anyhow::Result<Vec<(Self, String)>> {
        if content.is_empty() {
            return Ok(vec![]);
        }

        let mut requests = Vec::with_capacity(content.len());

        for file in content.iter() {
            let pathbuf = PathBuf::from_str(file)?;

            if !pathbuf.exists() {
                continue;
            }

            let filename = pathbuf
                .file_name()
                .unwrap()
                .to_str()
                .expect("OsString conversion")
                .to_string();

            let body = ShareRequestFile {
                filename,
                open: Some(false),
            };

            requests.push((Self::File(body), pathbuf.to_str().unwrap().to_string()));
        }

        Ok(requests)
    }

    pub async fn receive_share(
        &self,
        device: &Device,
        info: &PacketPayloadTransferInfo,
    ) -> anyhow::Result<()> {
        match self {
            ShareRequest::File(share_request_file) => self
                .handle_file_request(share_request_file, device, info)
                .await
                .expect("handling file share request"),
            ShareRequest::Text { text: _ } => todo!(),
            ShareRequest::Url { url: _ } => todo!(),
        }

        Ok(())
    }

    pub async fn handle_file_request(
        &self,
        request: &ShareRequestFile,
        device: &Device,
        info: &PacketPayloadTransferInfo,
    ) -> anyhow::Result<()> {
        let filename = request.filename.clone();
        let _open = request.open.unwrap_or(false);

        let file_accepted = tokio::task::spawn_blocking(move || {
            let mut file_accepted = false;

            let mut notify = notify_rust::Notification::new();
            notify.appname("KDE Connect");
            notify.summary("A remote device requested file share");
            notify.body(&format!(
                "A remote device wants to share file, {}, do you want to accept it?",
                &filename
            ));

            notify
                .action("default", "accepted")
                .hint(notify_rust::Hint::Resident(true))
                .timeout(Timeout::Never)
                .show()
                .unwrap()
                .wait_for_action(|action| match action {
                    "default" => {
                        file_accepted = true;
                    }
                    "__closed" => {
                        file_accepted = false;
                    }
                    _ => {
                        file_accepted = false;
                    }
                });

            file_accepted
        })
        .await?;

        if file_accepted {
            let current_name = request.filename.as_str();
            let temp_dir = PathBuf::from_str("/tmp")?;
            let temp_file = temp_dir.join(&request.filename);
            let mut remote_server = device.address;
            remote_server.set_port(info.port);
            let domain = device.device_id.clone();

            let _ = receive_payload(&domain, &remote_server, &temp_file).await;

            if let Some(home_dir) = dirs::download_dir() {
                let file = SelectedFiles::save_file()
                    .title("Save file")
                    .accept_label("Save")
                    .modal(true)
                    .current_folder(home_dir)?
                    .current_name(current_name)
                    .current_file(&temp_file)?
                    .send()
                    .await?
                    .response()?;

                let file_path: Option<PathBuf> = file
                    .uris()
                    .first()
                    .map(|path| path.to_file_path().unwrap_or_default());

                let Some(file_path) = file_path else {
                    return Ok(());
                };

                if tokio::fs::rename(temp_file, &file_path).await.is_ok() {
                    tokio::task::spawn_blocking(move || {
                        let mut notify = notify_rust::Notification::new();
                        notify.appname("KDE Connect");
                        notify.summary(&format!("File saved to: {:?}", file_path));

                        notify
                            .hint(notify_rust::Hint::Resident(true))
                            .show()
                            .unwrap();
                    })
                    .await
                    .unwrap();
                }
            }
        };

        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShareRequestFile {
    pub filename: String,
    pub open: Option<bool>,
}

impl Plugin for ShareRequest {
    fn id(&self) -> &'static str {
        "kdeconnect.share.request"
    }
}

impl ShareRequest {
    pub async fn send_file(
        &self,
        writer: &mpsc::UnboundedSender<ProtocolPacket>,
        payload_size: i64,
        payload_transfer_info: Option<PacketPayloadTransferInfo>,
    ) {
        let packet = ProtocolPacket::new_with_payload(
            PacketType::ShareRequest,
            serde_json::to_value(self.clone()).expect("failed serialize packet body"),
            payload_size,
            payload_transfer_info,
        );

        let _ = writer.send(packet);
    }
}
