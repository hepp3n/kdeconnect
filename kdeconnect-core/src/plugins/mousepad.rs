use serde::{Deserialize, Serialize};
use std::{
    io::Read as _,
    process::{Command, ExitStatus, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{
    device::{Device, DeviceId},
    event::CoreEvent,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
};

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct KeyboardState {
    pub state: Option<bool>,
}

impl Plugin for KeyboardState {
    fn id(&self) -> &'static str {
        "kdeconnect.mousepad.keyboardstate"
    }
}

static YDOTOOL_MISSING_WARNED: AtomicBool = AtomicBool::new(false);
static YDOTOOL_DAEMON_WARNED: AtomicBool = AtomicBool::new(false);
static YDOTOOL_TIMEOUT_WARNED: AtomicBool = AtomicBool::new(false);
static INPUT_QUEUE_FULL_WARNED: AtomicBool = AtomicBool::new(false);
static INPUT_QUEUE_CLOSED_WARNED: AtomicBool = AtomicBool::new(false);

const INPUT_QUEUE_CAPACITY: usize = 256;
const YDOTOOL_TIMEOUT: Duration = Duration::from_secs(2);

static INPUT_QUEUE: OnceLock<mpsc::Sender<InputJob>> = OnceLock::new();

#[derive(Debug)]
enum InputJob {
    Mousepad {
        request: MousepadRequest,
        device: DeviceId,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
    },
    Presenter(PresenterRequest),
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MousepadRequest {
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub scroll: Option<bool>,
    pub key: Option<String>,
    pub special_key: Option<u8>,
    pub alt: Option<bool>,
    pub ctrl: Option<bool>,
    pub shift: Option<bool>,
    #[serde(rename = "super")]
    pub meta: Option<bool>,
    pub singleclick: Option<bool>,
    pub doubleclick: Option<bool>,
    pub middleclick: Option<bool>,
    pub rightclick: Option<bool>,
    pub singlehold: Option<bool>,
    pub singlerelease: Option<bool>,
    #[serde(rename = "sendAck")]
    pub send_ack: Option<bool>,
    #[serde(rename = "isAck")]
    pub is_ack: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
pub struct PresenterRequest {
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub stop: Option<bool>,
}

impl MousepadRequest {
    pub async fn received_packet(
        &self,
        device: &Device,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
    ) {
        enqueue_input_job(InputJob::Mousepad {
            request: self.clone(),
            device: device.device_id.clone(),
            core_tx,
        });
    }
}

impl PresenterRequest {
    pub async fn received_packet(&self) {
        enqueue_input_job(InputJob::Presenter(self.clone()));
    }
}

fn input_queue() -> &'static mpsc::Sender<InputJob> {
    INPUT_QUEUE.get_or_init(|| {
        let (tx, mut rx) = mpsc::channel(INPUT_QUEUE_CAPACITY);

        tokio::spawn(async move {
            while let Some(job) = rx.recv().await {
                match job {
                    InputJob::Mousepad {
                        request,
                        device,
                        core_tx,
                    } => {
                        let request_for_run = request.clone();
                        let result = tokio::task::spawn_blocking(move || {
                            run_mousepad_request(&request_for_run)
                        })
                        .await;

                        match result {
                            Ok(Ok(())) => {
                                if request.send_ack == Some(true) {
                                    send_mousepad_ack(request, device, core_tx);
                                }
                            }
                            Ok(Err(error)) => {
                                debug!("[mousepad] failed to handle request: {error}");
                            }
                            Err(error) => warn!("[mousepad] handler task failed: {error}"),
                        }
                    }
                    InputJob::Presenter(request) => {
                        match tokio::task::spawn_blocking(move || run_presenter_request(&request))
                            .await
                        {
                            Ok(Ok(())) => {}
                            Ok(Err(error)) => {
                                debug!("[mousepad] failed to handle presenter request: {error}");
                            }
                            Err(error) => {
                                warn!("[mousepad] presenter handler task failed: {error}")
                            }
                        }
                    }
                }
            }
        });

        tx
    })
}

fn enqueue_input_job(job: InputJob) {
    match input_queue().try_send(job) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            if !INPUT_QUEUE_FULL_WARNED.swap(true, Ordering::Relaxed) {
                warn!(
                    "[mousepad] remote input queue is full; dropping input until the backend catches up"
                );
            }
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            if !INPUT_QUEUE_CLOSED_WARNED.swap(true, Ordering::Relaxed) {
                warn!("[mousepad] remote input queue is closed; dropping input");
            }
        }
    }
}

fn send_mousepad_ack(
    mut request: MousepadRequest,
    device: DeviceId,
    core_tx: mpsc::UnboundedSender<CoreEvent>,
) {
    request.send_ack = None;
    request.is_ack = Some(true);
    let packet = ProtocolPacket::new(
        PacketType::MousePadEcho,
        serde_json::to_value(request).unwrap_or_default(),
    );
    let _ = core_tx.send(CoreEvent::SendPacket { device, packet });
}

fn run_presenter_request(request: &PresenterRequest) -> anyhow::Result<()> {
    if request.stop.unwrap_or(false) {
        return Ok(());
    }

    if let (Some(dx), Some(dy)) = (request.dx, request.dy) {
        return ydotool([
            "mousemove",
            "--",
            &(dx * 1000.0).round().to_string(),
            &(dy * 1000.0).round().to_string(),
        ]);
    }

    Ok(())
}

fn run_mousepad_request(request: &MousepadRequest) -> anyhow::Result<()> {
    if let Some(scroll) = request.scroll {
        let dx = request.dx.unwrap_or_default();
        let dy = request.dy.unwrap_or_default();
        if scroll && (dx != 0.0 || dy != 0.0) {
            return ydotool([
                "mousemove",
                "-w",
                "--",
                &dx.round().to_string(),
                &dy.round().to_string(),
            ]);
        }
    }

    if let (Some(dx), Some(dy)) = (request.dx, request.dy) {
        return ydotool([
            "mousemove",
            "--",
            &dx.round().to_string(),
            &dy.round().to_string(),
        ]);
    }

    if request.singleclick.unwrap_or(false) {
        return ydotool(["click", "0xC0"]);
    }
    if request.doubleclick.unwrap_or(false) {
        ydotool(["click", "0xC0"])?;
        return ydotool(["click", "0xC0"]);
    }
    if request.middleclick.unwrap_or(false) {
        return ydotool(["click", "0xC2"]);
    }
    if request.rightclick.unwrap_or(false) {
        return ydotool(["click", "0xC1"]);
    }
    if request.singlehold.unwrap_or(false) {
        return ydotool(["click", "0x40"]);
    }
    if request.singlerelease.unwrap_or(false) {
        return ydotool(["click", "0x80"]);
    }

    if let Some(text) = request.key.as_deref() {
        if text == "\0" {
            return Ok(());
        }
        if !text.is_empty() {
            return ydotool(["type", "--", text]);
        }
    }

    if let Some(special_key) = request.special_key
        && let Some(code) = special_key_code(special_key)
    {
        return ydotool_key_with_modifiers(code, request);
    }

    debug!("[mousepad] ignored unsupported request: {:?}", request);
    Ok(())
}

fn ydotool_key_with_modifiers(code: u16, request: &MousepadRequest) -> anyhow::Result<()> {
    let mut keys = Vec::new();
    if request.ctrl.unwrap_or(false) {
        keys.push(29);
    }
    if request.alt.unwrap_or(false) {
        keys.push(56);
    }
    if request.shift.unwrap_or(false) {
        keys.push(42);
    }
    if request.meta.unwrap_or(false) {
        keys.push(125);
    }

    for modifier in &keys {
        ydotool(["key", &format!("{modifier}:1")])?;
    }
    ydotool(["key", &format!("{code}:1"), &format!("{code}:0")])?;
    for modifier in keys.iter().rev() {
        ydotool(["key", &format!("{modifier}:0")])?;
    }

    Ok(())
}

fn ydotool<const N: usize>(args: [&str; N]) -> anyhow::Result<()> {
    let mut child = match Command::new("ydotool")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !YDOTOOL_MISSING_WARNED.swap(true, Ordering::Relaxed) {
                warn!("[mousepad] ydotool is not installed; remote input is unavailable");
            }
            return Err(e.into());
        }
        Err(e) => return Err(e.into()),
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stderr = Vec::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_end(&mut stderr);
                }
                return handle_ydotool_status(status, &stderr);
            }
            Ok(None) if started.elapsed() >= YDOTOOL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                if !YDOTOOL_TIMEOUT_WARNED.swap(true, Ordering::Relaxed) {
                    warn!(
                        "[mousepad] ydotool did not exit within {}s; remote input backend appears stuck",
                        YDOTOOL_TIMEOUT.as_secs()
                    );
                }
                anyhow::bail!("ydotool timed out");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(e.into()),
        }
    }
}

fn handle_ydotool_status(status: ExitStatus, stderr: &[u8]) -> anyhow::Result<()> {
    if status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(stderr);
    let message = stderr.trim();
    if message.contains("ydotoold is running") || message.contains("failed to connect socket") {
        if !YDOTOOL_DAEMON_WARNED.swap(true, Ordering::Relaxed) {
            warn!(
                "[mousepad] ydotoold is not running or its socket is unavailable; remote input is unavailable"
            );
        }
    } else if message.is_empty() {
        warn!("[mousepad] ydotool exited with status {}", status);
    } else {
        warn!(
            "[mousepad] ydotool exited with status {}: {message}",
            status
        );
    }

    anyhow::bail!("ydotool exited with status {}", status)
}

fn special_key_code(key: u8) -> Option<u16> {
    Some(match key {
        1 => 14,
        2 => 15,
        3 => 101,
        4 => 105,
        5 => 103,
        6 => 106,
        7 => 108,
        8 => 104,
        9 => 109,
        10 => 102,
        11 => 107,
        12 => 28,
        13 => 111,
        14 => 1,
        15 => 99,
        16 => 70,
        21 => 59,
        22 => 60,
        23 => 61,
        24 => 62,
        25 => 63,
        26 => 64,
        27 => 65,
        28 => 66,
        29 => 67,
        30 => 68,
        31 => 87,
        32 => 88,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_camel_case_mousepad_fields() {
        let request: MousepadRequest = serde_json::from_value(serde_json::json!({
            "specialKey": 12,
            "sendAck": true,
            "super": true
        }))
        .unwrap();

        assert_eq!(request.special_key, Some(12));
        assert_eq!(request.send_ack, Some(true));
        assert_eq!(request.meta, Some(true));
    }

    #[test]
    fn maps_kdeconnect_special_keys_to_linux_input_codes() {
        assert_eq!(special_key_code(1), Some(14));
        assert_eq!(special_key_code(12), Some(28));
        assert_eq!(special_key_code(32), Some(88));
        assert_eq!(special_key_code(99), None);
    }
}
