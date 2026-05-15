use std::process::Stdio;

use percent_encoding::{AsciiSet, CONTROLS, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::device::Device;

const SFTP_PATH_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'#')
    .add(b'?')
    .add(b'{')
    .add(b'}');

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Sftp {
    pub ip: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub multi_paths: Vec<String>,
    #[serde(default)]
    pub path_names: Vec<String>,
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
            });
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

        let host = self
            .ip
            .as_deref()
            .filter(|ip| !ip.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| device.address.ip().to_string());

        let user = self.user.clone().unwrap_or_default();
        let password = self.password.clone().unwrap_or_default();
        let rpath = self.path.clone().unwrap_or_default();
        let device_name = device.name.clone();

        tokio::task::spawn_blocking(move || {
            mount_and_open(&host, port, &user, &password, &rpath, &device_name);
        });
    }
}

fn sftp_uri(
    host: &str,
    port: u16,
    user: Option<&str>,
    password: Option<&str>,
    path: Option<&str>,
) -> String {
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

    let path = path
        .filter(|path| !path.is_empty())
        .map(|path| {
            if path.starts_with('/') {
                path.to_string()
            } else {
                format!("/{path}")
            }
        })
        .unwrap_or_else(|| "/".to_string());
    let path = utf8_percent_encode(&path, SFTP_PATH_ENCODE_SET);

    let host = bracket_ipv6_host(host);
    format!("sftp://{}{}:{}{}", auth, host, port, path)
}

fn bracket_ipv6_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
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

fn mount_and_open(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    rpath: &str,
    device_name: &str,
) {
    let path = if rpath.is_empty() { "/" } else { rpath };
    let host_bracketed = bracket_ipv6_host(host);
    let remote = format!("{}@{}:{}", user, host_bracketed, path);

    info!("[sftp] mounting {} on port {}", host_bracketed, port);

    if try_gio_mount(host, port, user, rpath)
        || try_sshfs_mount(&remote, port, password, device_name)
    {
        return;
    }

    let uri = sftp_uri(host, port, Some(user), Some(password), Some(path));
    let display_uri = uri_without_password(&uri);
    let _ = notify_rust::Notification::new()
        .appname("KDE Connect")
        .summary(&format!("{} filesystem", device_name))
        .body(&format!("Could not mount.\nManual access: {}", display_uri))
        .show();
}

fn try_gio_mount(host: &str, port: u16, user: &str, rpath: &str) -> bool {
    let path = if rpath.is_empty() { "/" } else { rpath };
    let uri = format!("sftp://{user}@{host}:{port}{path}");

    let status = std::process::Command::new("gio")
        .args(["mount", &uri])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if status.map(|s| s.success()).unwrap_or(false) {
        info!("[sftp] gio mount succeeded for {user}@{host}:{port}");

        let _ = std::process::Command::new("gio")
            .args(["open", &uri])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        return true;
    }

    false
}

fn try_sshfs_mount(remote: &str, port: u16, password: &str, device_name: &str) -> bool {
    let Some(runtime_dir) = dirs::runtime_dir()
        .or_else(|| {
            std::env::var("XDG_RUNTIME_DIR")
                .ok()
                .map(std::path::PathBuf::from)
        })
        .or_else(|| dirs::cache_dir())
    else {
        warn!("[sftp] cannot determine runtime directory for mount point");
        return false;
    };

    let mount_dir = runtime_dir.join("kdeconnect-sftp");
    let _ = std::fs::create_dir_all(&mount_dir);

    let clean_name = device_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mount_point = mount_dir.join(&clean_name);

    if mount_point.exists() {
        if is_mounted(&mount_point) {
            open_directory(&mount_point);
            return true;
        }
        let _ = std::fs::remove_dir(&mount_point);
    }

    let _ = std::fs::create_dir(&mount_point);

    let mut cmd = std::process::Command::new("sshfs");
    cmd.arg(&remote)
        .arg(&mount_point)
        .arg("-p")
        .arg(port.to_string())
        .arg("-o")
        .arg("StrictHostKeyChecking=no")
        .arg("-o")
        .arg("UserKnownHostsFile=/dev/null")
        .arg("-o")
        .arg("reconnect")
        .arg("-o")
        .arg("ServerAliveInterval=30")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let status = cmd.status();

    if status.map(|s| s.success()).unwrap_or(false) {
        info!(
            "[sftp] sshfs mounted {} at {}",
            remote,
            mount_point.display()
        );
        open_directory(&mount_point);
        return true;
    }

    if !password.is_empty() {
        let mut cmd = std::process::Command::new("sshfs");
        cmd.arg(&remote)
            .arg(&mount_point)
            .arg("-p")
            .arg(port.to_string())
            .arg("-o")
            .arg("StrictHostKeyChecking=no")
            .arg("-o")
            .arg("UserKnownHostsFile=/dev/null")
            .arg("-o")
            .arg("reconnect")
            .arg("-o")
            .arg("ServerAliveInterval=30")
            .arg("-o")
            .arg("ConnectTimeout=10")
            .arg("-o")
            .arg("password_stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if let Ok(mut child) = cmd.spawn() {
            use std::io::Write;
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(password.as_bytes());
                let _ = stdin.write_all(b"\n");
            }
            let status = child.wait();
            if status.map(|s| s.success()).unwrap_or(false) {
                info!(
                    "[sftp] sshfs (password) mounted {} at {}",
                    remote,
                    mount_point.display()
                );
                open_directory(&mount_point);
                return true;
            }
        }
    }

    let _ = std::fs::remove_dir(&mount_point);
    warn!("[sftp] sshfs failed to mount {}", remote);
    false
}

fn is_mounted(path: &std::path::Path) -> bool {
    let output = std::process::Command::new("mountpoint")
        .arg("-q")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    output.map(|s| s.success()).unwrap_or(false)
}

fn open_directory(path: &std::path::Path) {
    let _ = std::process::Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
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
            sftp_uri(
                "192.168.1.10",
                1740,
                Some("kde connect"),
                Some("pa:ss"),
                Some("/storage/emulated/0")
            ),
            "sftp://kde%20connect:pa%3Ass@192.168.1.10:1740/storage/emulated/0"
        );
    }

    #[test]
    fn builds_sftp_uri_with_relative_path_and_ipv6_host() {
        assert_eq!(
            sftp_uri("2001:db8::1", 1740, Some("kdeconnect"), None, Some("DCIM")),
            "sftp://kdeconnect@[2001:db8::1]:1740/DCIM"
        );
    }

    #[test]
    fn builds_sftp_uri_with_encoded_path() {
        assert_eq!(
            sftp_uri(
                "192.168.1.10",
                1740,
                Some("kdeconnect"),
                None,
                Some("/Camera Pictures")
            ),
            "sftp://kdeconnect@192.168.1.10:1740/Camera%20Pictures"
        );
    }

    #[test]
    fn redacts_password_for_display() {
        assert_eq!(
            uri_without_password("sftp://user:secret@192.168.1.10:1740/"),
            "sftp://user:***@192.168.1.10:1740/"
        );
    }

    #[test]
    fn deserializes_full_sftp_response_with_path_names() {
        use super::Sftp;
        let json = serde_json::json!({
            "ip": "192.168.1.71",
            "port": 1743,
            "user": "kdeconnect",
            "password": "abc123",
            "path": "/storage/emulated/0",
            "multiPaths": ["/storage/0000-0000", "/storage/emulated/0"],
            "pathNames": ["SD Card", "All files"]
        });
        let sftp: Sftp = serde_json::from_value(json).unwrap();
        assert_eq!(sftp.ip.as_deref(), Some("192.168.1.71"));
        assert_eq!(sftp.port, Some(1743));
        assert_eq!(sftp.multi_paths.len(), 2);
        assert_eq!(sftp.path_names.len(), 2);
        assert_eq!(sftp.path_names[0], "SD Card");
        assert_eq!(sftp.path_names[1], "All files");
    }

    #[test]
    fn received_packet_returns_immediately_without_blocking() {
        use crate::device::{Device, DeviceId};
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let sftp_err = super::Sftp {
                ip: None,
                port: None,
                user: None,
                password: None,
                path: None,
                multi_paths: vec![],
                path_names: vec![],
                error_message: Some("test error".to_string()),
            };
            let device = Device {
                device_id: DeviceId("test-sftp-nonblock".into()),
                ..Default::default()
            };

            tokio::time::timeout(
                Duration::from_millis(500),
                sftp_err.received_packet(&device),
            )
            .await
            .is_ok()
        });

        rt.shutdown_background();

        assert!(
            passed,
            "sftp received_packet must return immediately without blocking on notification"
        );
    }
}
