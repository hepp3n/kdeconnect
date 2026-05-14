use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

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
                warn!("[share] skipping missing path: {}", pathbuf.display());
                continue;
            }

            let Some(filename) = pathbuf
                .file_name()
                .and_then(|name| name.to_str())
                .map(ToOwned::to_owned)
            else {
                warn!(
                    "[share] skipping path without valid UTF-8 filename: {}",
                    pathbuf.display()
                );
                continue;
            };

            let Some(path) = pathbuf.to_str().map(ToOwned::to_owned) else {
                warn!("[share] skipping non-UTF-8 path: {}", pathbuf.display());
                continue;
            };

            let body = ShareRequestFile {
                filename,
                open: Some(false),
            };

            requests.push((Self::File(body), path));
        }

        Ok(requests)
    }

    pub async fn receive_share(
        &self,
        device: &Device,
        info: Option<&PacketPayloadTransferInfo>,
    ) -> anyhow::Result<()> {
        match self {
            ShareRequest::File(f) => {
                let Some(info) = info else {
                    return Err(anyhow::anyhow!(
                        "file share from {} did not include payload transfer info",
                        device.name
                    ));
                };
                self.handle_file_request(f, device, info).await
            }
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
                    if std::process::Command::new("xdg-open")
                        .arg(&url)
                        .spawn()
                        .is_err()
                    {
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

        if request.open.unwrap_or(false) {
            let open_path = dest.clone();
            tokio::task::spawn_blocking(move || {
                let _ = std::process::Command::new("xdg-open")
                    .arg(open_path)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
            })
            .await
            .ok();
        }

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

fn sanitize_filename(filename: &str) -> String {
    let after_last_sep = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let sanitized: String = after_last_sep
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0'))
        .collect();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "download".to_string()
    } else {
        sanitized
    }
}

fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let filename = sanitize_filename(filename);
    let candidate = dir.join(&filename);
    if !candidate.exists() {
        return candidate;
    }

    let stem = PathBuf::from(&filename)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| filename.clone());
    let ext = PathBuf::from(&filename)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    for i in 1u32..10000 {
        let candidate = dir.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }

    dir.join(&filename)
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
        payload_size: u64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_directory_traversal() {
        assert_eq!(
            sanitize_filename("../../.ssh/authorized_keys"),
            "authorized_keys"
        );
        assert_eq!(sanitize_filename("../../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/etc/shadow"), "shadow");
    }

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_filename("foo/bar/baz.txt"), "baz.txt");
        assert_eq!(sanitize_filename("foo\\bar\\baz.txt"), "baz.txt");
    }

    #[test]
    fn sanitize_handles_empty_and_dot_names() {
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("."), "download");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename("/"), "download");
        assert_eq!(sanitize_filename("///"), "download");
    }

    #[test]
    fn sanitize_preserves_normal_filenames() {
        assert_eq!(sanitize_filename("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_filename("my file (1).txt"), "my file (1).txt");
        assert_eq!(sanitize_filename("document.tar.gz"), "document.tar.gz");
    }

    #[test]
    fn sanitize_handles_null_bytes() {
        assert_eq!(sanitize_filename("evil\0name.txt"), "evilname.txt");
    }

    #[test]
    fn unique_path_uses_sanitized_name() {
        let dir = std::env::temp_dir();
        let result = unique_path(&dir, "../../etc/passwd");
        assert!(result.starts_with(&dir));
        assert_eq!(result.file_name().unwrap().to_str().unwrap(), "passwd");
    }

    #[test]
    fn unique_path_returns_base_if_not_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let result = unique_path(tmp.path(), "test.txt");
        assert_eq!(result, tmp.path().join("test.txt"));
    }

    #[test]
    fn unique_path_appends_counter_for_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("test.txt"), "").unwrap();
        let result = unique_path(tmp.path(), "test.txt");
        assert_eq!(result, tmp.path().join("test (1).txt"));
    }

    #[test]
    fn unique_path_skips_existing_counters() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("test.txt"), "").unwrap();
        std::fs::write(tmp.path().join("test (1).txt"), "").unwrap();
        std::fs::write(tmp.path().join("test (2).txt"), "").unwrap();
        let result = unique_path(tmp.path(), "test.txt");
        assert_eq!(result, tmp.path().join("test (3).txt"));
    }
}
