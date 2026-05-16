use std::{
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
};

use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::device::Device;

static ALARM_ACTIVE: AtomicBool = AtomicBool::new(false);

pub async fn handle_request(device: &Device) {
    let name = device.name.clone();
    tokio::task::spawn_blocking(move || {
        let _ = notify_rust::Notification::new()
            .appname("KDE Connect")
            .summary("Find My Device")
            .body(&format!("{} is trying to find this computer.", name))
            .show();
    });

    if ALARM_ACTIVE.swap(true, Ordering::AcqRel) {
        debug!("[findmyphone] alarm already active, skipping duplicate");
        return;
    }

    tokio::spawn(async move {
        unmute_audio().await;
        let played = play_alarm().await;
        remute_audio().await;
        ALARM_ACTIVE.store(false, Ordering::Release);

        if !played {
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

async fn unmute_audio() {
    let was_muted = is_sink_muted().await;

    if was_muted {
        info!("[findmyphone] unmuting audio for alarm");
        let _ = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        let _ = Command::new("wpctl")
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", "100%"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        let _ = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", "0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
}

async fn remute_audio() {
    info!("[findmyphone] re-muting audio after alarm");
    let _ = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await;
}

async fn is_sink_muted() -> bool {
    let output = Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;

    match output {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout);
            text.contains("[MUTED]")
        }
        Err(_) => false,
    }
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

    #[test]
    fn duplicate_requests_do_not_stack_alarms() {
        ALARM_ACTIVE.store(false, Ordering::Release);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let device = Device {
                device_id: DeviceId("test-findmyphone-dedup".into()),
                name: "Test Phone".into(),
                ..Default::default()
            };

            handle_request(&device).await;

            let was_active_before = ALARM_ACTIVE.load(Ordering::Acquire);
            assert!(was_active_before, "first request should set alarm active");

            handle_request(&device).await;
        });

        rt.shutdown_background();
        ALARM_ACTIVE.store(false, Ordering::Release);
    }
}
