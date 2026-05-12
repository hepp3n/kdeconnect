use std::process::Stdio;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::device::Device;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Sftp {
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub multi_paths: Vec<String>,
    pub error_message: Option<String>,
}

impl Sftp {
    pub async fn received_packet(self, device: &Device) {
        if let Some(error) = self.error_message {
            let name = device.name.clone();
            tokio::task::spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .appname("KDE Connect")
                    .summary(&format!("{} reported an SFTP error", name))
                    .body(&error)
                    .show();
            })
            .await
            .ok();
            return;
        }

        let Some(port) = self.port else {
            warn!("[sftp] missing port in mount response from {}", device.name);
            return;
        };

        if let Some(user) = self.user.as_deref() {
            add_private_key().await;
            debug!("[sftp] response includes user '{}'", user);
        }

        let uri = sftp_uri(
            &device.address.ip().to_string(),
            port,
            self.user.as_deref(),
            self.password.as_deref(),
        );
        info!("[sftp] opening {}", uri_without_password(&uri));

        let opened = run_uri_command("gio", &["open", &uri]).await
            || run_uri_command("xdg-open", &[&uri]).await;

        if !opened {
            let display_uri = uri_without_password(&uri);
            tokio::task::spawn_blocking(move || {
                let _ = notify_rust::Notification::new()
                    .appname("KDE Connect")
                    .summary("Device filesystem is ready")
                    .body(&display_uri)
                    .show();
            })
            .await
            .ok();
        }
    }
}

fn sftp_uri(host: &str, port: u16, user: Option<&str>, password: Option<&str>) -> String {
    let auth = match (user, password) {
        (Some(user), Some(password)) if !user.is_empty() => format!(
            "{}:{}@",
            utf8_percent_encode(user, NON_ALPHANUMERIC),
            utf8_percent_encode(password, NON_ALPHANUMERIC)
        ),
        (Some(user), _) if !user.is_empty() => {
            format!("{}@", utf8_percent_encode(user, NON_ALPHANUMERIC))
        }
        _ => String::new(),
    };

    format!("sftp://{}{}:{}/", auth, host, port)
}

fn uri_without_password(uri: &str) -> String {
    if let Some((scheme, rest)) = uri.split_once("://")
        && let Some(at) = rest.find('@')
        && let Some(colon) = rest[..at].find(':')
    {
        return format!("{}://{}:***@{}", scheme, &rest[..colon], &rest[at + 1..]);
    }
    uri.to_string()
}

async fn run_uri_command(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| true)
        .unwrap_or(false)
}

async fn add_private_key() {
    let Some(config_dir) = dirs::config_dir().map(|d| d.join("kdeconnect")) else {
        return;
    };

    for key in ["private.pem", "privateKey.pem"] {
        let path = config_dir.join(key);
        if !path.exists() {
            continue;
        }

        let status = Command::new("ssh-add")
            .arg(&path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        if let Ok(status) = status
            && status.success()
        {
            debug!("[sftp] added private key {}", path.display());
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{sftp_uri, uri_without_password};

    #[test]
    fn builds_sftp_uri_with_escaped_credentials() {
        assert_eq!(
            sftp_uri("192.168.1.10", 1740, Some("kde connect"), Some("pa:ss")),
            "sftp://kde%20connect:pa%3Ass@192.168.1.10:1740/"
        );
    }

    #[test]
    fn redacts_password_for_display() {
        assert_eq!(
            uri_without_password("sftp://user:secret@192.168.1.10:1740/"),
            "sftp://user:***@192.168.1.10:1740/"
        );
    }
}
