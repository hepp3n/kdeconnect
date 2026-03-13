use std::{path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};

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
            ShareRequest::File(f) => self.handle_file_request(f, device, info).await,
            ShareRequest::Text { text } => {
                let text = text.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = notify_rust::Notification::new()
                        .appname("KDE Connect")
                        .summary("Text received from phone")
                        .body(&text)
                        .show();
                })
                .await
                .ok();
                Ok(())
            }
            ShareRequest::Url { url } => {
                let url = url.clone();
                tokio::task::spawn_blocking(move || {
                    if std::process::Command::new("xdg-open").arg(&url).spawn().is_err() {
                        let _ = notify_rust::Notification::new()
                            .appname("KDE Connect")
                            .summary("URL received from phone")
                            .body(&url)
                            .show();
                    }
                })
                .await
                .ok();
                Ok(())
            }
        }
    }

    async fn handle_file_request(
        &self,
        request: &ShareRequestFile,
        device: &Device,
        info: &PacketPayloadTransferInfo,
    ) -> anyhow::Result<()> {
        let download_dir = dirs::download_dir().unwrap_or_else(|| {
            warn!("[share] cannot find Downloads dir, falling back to /tmp");
            PathBuf::from("/tmp")
        });

        // Avoid overwriting existing files by appending a counter if needed.
        let dest = unique_path(&download_dir, &request.filename);

        let mut remote_addr = device.address;
        remote_addr.set_port(info.port);
        let domain = device.device_id.clone();

        info!(
            "[share] receiving '{}' from {} ({}:{})",
            request.filename,
            device.name,
            remote_addr.ip(),
            info.port
        );

        if let Err(e) = receive_payload(&domain, &remote_addr, &dest).await {
            warn!(
                "[share] receive_payload failed for '{}': {}",
                request.filename, e
            );
            return Err(e);
        }

        info!("[share] saved '{}' to {:?}", request.filename, dest);

        let dest_display = dest.display().to_string();
        let filename = request.filename.clone();
        tokio::task::spawn_blocking(move || {
            let _ = notify_rust::Notification::new()
                .appname("KDE Connect")
                .summary(&format!("File received: {}", filename))
                .body(&dest_display)
                .show();
        })
        .await
        .ok();

        Ok(())
    }
}

/// Returns a path that doesn't already exist by appending ` (N)` before the
/// extension — e.g. `photo (1).jpg`, `photo (2).jpg`.
fn unique_path(dir: &PathBuf, filename: &str) -> PathBuf {
    let candidate = dir.join(filename);
    if !candidate.exists() {
        return candidate;
    }

    let stem = PathBuf::from(filename)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| filename.to_string());
    let ext = PathBuf::from(filename)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    for i in 1u32.. {
        let candidate = dir.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(filename)
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
