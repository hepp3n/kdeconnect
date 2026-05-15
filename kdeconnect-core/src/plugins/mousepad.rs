use serde::{Deserialize, Serialize};
use std::{
    io::Read as _,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Mutex as StdMutex, OnceLock,
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
static MOTION_FLUSH_QUEUED: AtomicBool = AtomicBool::new(false);

const INPUT_QUEUE_CAPACITY: usize = 256;
const YDOTOOL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_MOTION_BATCHES_PER_FLUSH: usize = 8;

static INPUT_QUEUE: OnceLock<mpsc::Sender<InputJob>> = OnceLock::new();
static MOTION_ACCUMULATOR: OnceLock<StdMutex<MotionAccumulator>> = OnceLock::new();

#[derive(Debug)]
enum InputJob {
    Mousepad {
        request: MousepadRequest,
        device: DeviceId,
        core_tx: mpsc::UnboundedSender<CoreEvent>,
    },
    FlushMouseMotion,
    Presenter(PresenterRequest),
}

#[derive(Debug, Default)]
struct MotionAccumulator {
    pointer_dx: f64,
    pointer_dy: f64,
    scroll_dx: f64,
    scroll_dy: f64,
}

#[derive(Debug, Default)]
struct MotionBatch {
    pointer_dx: i64,
    pointer_dy: i64,
    scroll_dx: i64,
    scroll_dy: i64,
}

impl MotionAccumulator {
    fn add_request(&mut self, request: &MousepadRequest) {
        let dx = request.dx.unwrap_or_default();
        let dy = request.dy.unwrap_or_default();
        if request.scroll.unwrap_or(false) {
            self.scroll_dx += dx;
            self.scroll_dy += dy;
        } else {
            self.pointer_dx += dx;
            self.pointer_dy += dy;
        }
    }

    fn add_presenter_request(&mut self, request: &PresenterRequest) {
        self.pointer_dx += request.dx.unwrap_or_default() * 1000.0;
        self.pointer_dy += request.dy.unwrap_or_default() * 1000.0;
    }

    fn has_dispatchable_motion(&self) -> bool {
        self.pointer_dx.round() != 0.0
            || self.pointer_dy.round() != 0.0
            || self.scroll_dx.round() != 0.0
            || self.scroll_dy.round() != 0.0
    }

    fn take_dispatchable_batch(&mut self) -> MotionBatch {
        let batch = MotionBatch {
            pointer_dx: self.pointer_dx.round() as i64,
            pointer_dy: self.pointer_dy.round() as i64,
            scroll_dx: self.scroll_dx.round() as i64,
            scroll_dy: self.scroll_dy.round() as i64,
        };

        self.pointer_dx -= batch.pointer_dx as f64;
        self.pointer_dy -= batch.pointer_dy as f64;
        self.scroll_dx -= batch.scroll_dx as f64;
        self.scroll_dy -= batch.scroll_dy as f64;

        batch
    }
}

impl MotionBatch {
    fn is_empty(&self) -> bool {
        self.pointer_dx == 0 && self.pointer_dy == 0 && self.scroll_dx == 0 && self.scroll_dy == 0
    }
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
        if self.is_coalescable_motion() {
            enqueue_motion_request(self);
            return;
        }

        enqueue_input_job(InputJob::Mousepad {
            request: self.clone(),
            device: device.device_id.clone(),
            core_tx,
        });
    }

    fn is_coalescable_motion(&self) -> bool {
        let has_motion = self.dx.unwrap_or_default() != 0.0 || self.dy.unwrap_or_default() != 0.0;
        has_motion
            && self.key.is_none()
            && self.special_key.is_none()
            && !self.alt.unwrap_or(false)
            && !self.ctrl.unwrap_or(false)
            && !self.shift.unwrap_or(false)
            && !self.meta.unwrap_or(false)
            && !self.singleclick.unwrap_or(false)
            && !self.doubleclick.unwrap_or(false)
            && !self.middleclick.unwrap_or(false)
            && !self.rightclick.unwrap_or(false)
            && !self.singlehold.unwrap_or(false)
            && !self.singlerelease.unwrap_or(false)
            && !self.send_ack.unwrap_or(false)
            && !self.is_ack.unwrap_or(false)
    }
}

impl PresenterRequest {
    pub async fn received_packet(&self) {
        if self.is_coalescable_motion() {
            enqueue_presenter_motion(self);
            return;
        }

        enqueue_input_job(InputJob::Presenter(self.clone()));
    }

    fn is_coalescable_motion(&self) -> bool {
        (self.dx.unwrap_or_default() != 0.0 || self.dy.unwrap_or_default() != 0.0)
            && !self.stop.unwrap_or(false)
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
                    InputJob::FlushMouseMotion => {
                        flush_mouse_motion().await;
                    }
                    InputJob::Presenter(request) => {
                        if request.stop.unwrap_or(false) {
                            flush_mouse_motion().await;
                        }

                        if request.dx.unwrap_or_default() != 0.0
                            || request.dy.unwrap_or_default() != 0.0
                        {
                            match tokio::task::spawn_blocking(move || {
                                run_presenter_request(&request)
                            })
                            .await
                            {
                                Ok(Ok(())) => {}
                                Ok(Err(error)) => {
                                    debug!("[mousepad] failed presenter request: {error}");
                                }
                                Err(error) => {
                                    warn!("[mousepad] presenter task failed: {error}")
                                }
                            }
                        }
                    }
                }
            }
        });

        tx
    })
}

fn enqueue_input_job(job: InputJob) -> bool {
    match input_queue().try_send(job) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => {
            if !INPUT_QUEUE_FULL_WARNED.swap(true, Ordering::Relaxed) {
                warn!(
                    "[mousepad] remote input queue is full; dropping input until the backend catches up"
                );
            }
            false
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            if !INPUT_QUEUE_CLOSED_WARNED.swap(true, Ordering::Relaxed) {
                warn!("[mousepad] remote input queue is closed; dropping input");
            }
            false
        }
    }
}

fn motion_accumulator() -> &'static StdMutex<MotionAccumulator> {
    MOTION_ACCUMULATOR.get_or_init(|| StdMutex::new(MotionAccumulator::default()))
}

fn enqueue_motion_request(request: &MousepadRequest) {
    let should_flush = {
        let mut guard = motion_accumulator().lock().unwrap();
        guard.add_request(request);
        guard.has_dispatchable_motion()
    };

    if should_flush
        && !MOTION_FLUSH_QUEUED.swap(true, Ordering::AcqRel)
        && !enqueue_input_job(InputJob::FlushMouseMotion)
    {
        MOTION_FLUSH_QUEUED.store(false, Ordering::Release);
    }
}

fn enqueue_presenter_motion(request: &PresenterRequest) {
    let should_flush = {
        let mut guard = motion_accumulator().lock().unwrap();
        guard.add_presenter_request(request);
        guard.has_dispatchable_motion()
    };

    if should_flush
        && !MOTION_FLUSH_QUEUED.swap(true, Ordering::AcqRel)
        && !enqueue_input_job(InputJob::FlushMouseMotion)
    {
        MOTION_FLUSH_QUEUED.store(false, Ordering::Release);
    }
}

fn take_motion_batch() -> MotionBatch {
    motion_accumulator()
        .lock()
        .unwrap()
        .take_dispatchable_batch()
}

fn has_dispatchable_motion() -> bool {
    motion_accumulator()
        .lock()
        .unwrap()
        .has_dispatchable_motion()
}

async fn flush_mouse_motion() {
    for _ in 0..MAX_MOTION_BATCHES_PER_FLUSH {
        let batch = take_motion_batch();
        if batch.is_empty() {
            break;
        }

        match tokio::task::spawn_blocking(move || run_motion_batch(&batch)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => debug!("[mousepad] failed to handle motion batch: {error}"),
            Err(error) => warn!("[mousepad] motion handler task failed: {error}"),
        }
    }

    MOTION_FLUSH_QUEUED.store(false, Ordering::Release);
    if has_dispatchable_motion()
        && !MOTION_FLUSH_QUEUED.swap(true, Ordering::AcqRel)
        && !enqueue_input_job(InputJob::FlushMouseMotion)
    {
        MOTION_FLUSH_QUEUED.store(false, Ordering::Release);
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
    let dx = request.dx.unwrap_or_default();
    let dy = request.dy.unwrap_or_default();

    if dx != 0.0 || dy != 0.0 {
        ydotool([
            "mousemove",
            "--",
            &(dx * 1000.0).round().to_string(),
            &(dy * 1000.0).round().to_string(),
        ])?;
    }

    Ok(())
}

fn run_motion_batch(batch: &MotionBatch) -> anyhow::Result<()> {
    if batch.pointer_dx != 0 || batch.pointer_dy != 0 {
        let dx = batch.pointer_dx.to_string();
        let dy = batch.pointer_dy.to_string();
        ydotool(["mousemove", "--", &dx, &dy])?;
    }

    if batch.scroll_dx != 0 || batch.scroll_dy != 0 {
        let dx = batch.scroll_dx.to_string();
        let dy = batch.scroll_dy.to_string();
        ydotool(["mousemove", "-w", "--", &dx, &dy])?;
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

    #[test]
    fn coalesces_plain_motion_but_not_clicks_or_acks() {
        assert!(
            MousepadRequest {
                dx: Some(1.0),
                dy: Some(2.0),
                ..Default::default()
            }
            .is_coalescable_motion()
        );

        assert!(
            MousepadRequest {
                dx: Some(0.0),
                dy: Some(2.0),
                scroll: Some(true),
                ..Default::default()
            }
            .is_coalescable_motion()
        );

        assert!(
            !MousepadRequest {
                dx: Some(1.0),
                dy: Some(2.0),
                singleclick: Some(true),
                ..Default::default()
            }
            .is_coalescable_motion()
        );

        assert!(
            !MousepadRequest {
                dx: Some(1.0),
                dy: Some(2.0),
                send_ack: Some(true),
                ..Default::default()
            }
            .is_coalescable_motion()
        );
    }

    #[test]
    fn parses_presenter_schema_and_coalesces_motion_only() {
        let motion: PresenterRequest = serde_json::from_value(serde_json::json!({
            "dx": 0.01,
            "dy": -0.02
        }))
        .unwrap();
        assert_eq!(motion.dx, Some(0.01));
        assert_eq!(motion.dy, Some(-0.02));
        assert!(motion.is_coalescable_motion());

        let stop: PresenterRequest = serde_json::from_value(serde_json::json!({
            "stop": true
        }))
        .unwrap();
        assert_eq!(stop.stop, Some(true));
        assert!(!stop.is_coalescable_motion());
    }

    #[test]
    fn motion_accumulator_keeps_fractional_remainder() {
        let mut accumulator = MotionAccumulator::default();
        accumulator.add_request(&MousepadRequest {
            dx: Some(0.4),
            dy: Some(0.4),
            ..Default::default()
        });
        assert!(!accumulator.has_dispatchable_motion());

        accumulator.add_request(&MousepadRequest {
            dx: Some(0.4),
            dy: Some(0.4),
            ..Default::default()
        });
        assert!(accumulator.has_dispatchable_motion());

        let batch = accumulator.take_dispatchable_batch();
        assert_eq!(batch.pointer_dx, 1);
        assert_eq!(batch.pointer_dy, 1);
        assert!(!accumulator.has_dispatchable_motion());
    }

    #[test]
    fn presenter_motion_reuses_mouse_motion_batching() {
        let mut accumulator = MotionAccumulator::default();
        accumulator.add_presenter_request(&PresenterRequest {
            dx: Some(0.0014),
            dy: Some(-0.0026),
            stop: None,
        });

        assert!(accumulator.has_dispatchable_motion());
        let batch = accumulator.take_dispatchable_batch();
        assert_eq!(batch.pointer_dx, 1);
        assert_eq!(batch.pointer_dy, -3);
        assert_eq!(batch.scroll_dx, 0);
        assert_eq!(batch.scroll_dy, 0);
    }

    #[test]
    fn presenter_stop_with_motion_is_not_coalescable() {
        let stop_with_motion = PresenterRequest {
            dx: Some(0.01),
            dy: Some(-0.02),
            stop: Some(true),
        };
        assert!(!stop_with_motion.is_coalescable_motion());

        let stop_only = PresenterRequest {
            dx: None,
            dy: None,
            stop: Some(true),
        };
        assert!(!stop_only.is_coalescable_motion());
    }

    #[test]
    fn presenter_accumulator_scales_by_thousand() {
        let mut accumulator = MotionAccumulator::default();
        accumulator.add_presenter_request(&PresenterRequest {
            dx: Some(1.5),
            dy: Some(-0.003),
            stop: None,
        });
        assert!(accumulator.has_dispatchable_motion());
        let batch = accumulator.take_dispatchable_batch();
        assert_eq!(batch.pointer_dx, 1500);
        assert_eq!(batch.pointer_dy, -3);
    }

    #[test]
    fn presenter_received_packet_returns_immediately() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let request = PresenterRequest {
                dx: Some(0.5),
                dy: Some(-0.3),
                ..Default::default()
            };

            tokio::time::timeout(Duration::from_millis(500), request.received_packet())
                .await
                .is_ok()
        });

        rt.shutdown_background();

        assert!(passed, "presenter received_packet must return immediately");
    }

    #[test]
    fn presenter_stop_received_packet_returns_immediately() {
        use std::time::Duration;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let passed = rt.block_on(async {
            let request = PresenterRequest {
                dx: None,
                dy: None,
                stop: Some(true),
            };

            tokio::time::timeout(Duration::from_millis(500), request.received_packet())
                .await
                .is_ok()
        });

        rt.shutdown_background();

        assert!(
            passed,
            "presenter stop received_packet must return immediately"
        );
    }
}
