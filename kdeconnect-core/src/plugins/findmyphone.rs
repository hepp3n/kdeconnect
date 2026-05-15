use std::process::Stdio;

use tokio::process::Command;
use tracing::{debug, warn};

use crate::device::Device;

pub async fn handle_request(device: &Device) {
    let name = device.name.clone();
    tokio::task::spawn_blocking(move || {
        let _ = notify_rust::Notification::new()
            .appname("KDE Connect")
            .summary("Find My Device")
            .body(&format!("{} is trying to find this computer.", name))
            .show();
    });

    tokio::spawn(async {
        if !play_alarm().await {
            warn!("[findmyphone] no supported sound player found; notification was shown");
        }
    });
}

async fn play_alarm() -> bool {
    let commands: &[(&str, &[&str])] = &[
        ("canberra-gtk-play", &["-i", "phone-incoming-call"]),
        ("canberra-gtk-play", &["-i", "alarm-clock-elapsed"]),
        (
            "pw-play",
            &["/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga"],
        ),
        (
            "paplay",
            &["/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga"],
        ),
    ];

    for (program, args) in commands {
        if command_exists(program).await {
            for _ in 0..10 {
                match Command::new(program)
                    .args(*args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await
                {
                    Ok(status) if status.success() => {}
                    Ok(status) => debug!("[findmyphone] {} exited with {}", program, status),
                    Err(e) => debug!("[findmyphone] failed to run {}: {}", program, e),
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            return true;
        }
    }

    false
}

async fn command_exists(program: &str) -> bool {
    Command::new("which")
        .arg(program)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Device, DeviceId};
    use std::time::Duration;

    #[test]
    fn handle_request_returns_immediately_without_blocking() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let device = Device {
                device_id: DeviceId("test-findmyphone-nonblock".into()),
                name: "Test Phone".into(),
                ..Default::default()
            };

            tokio::time::timeout(Duration::from_millis(500), handle_request(&device))
                .await
                .is_ok()
        });

        rt.shutdown_background();

        assert!(
            passed,
            "findmyphone handle_request must return immediately without blocking on notification"
        );
    }
}
