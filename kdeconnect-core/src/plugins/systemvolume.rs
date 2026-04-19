use libpulse_binding::{
    callbacks::ListResult,
    context::{
        Context, FlagSet as ContextFlagSet, State as ContextState,
        introspect::SinkInfo as PASinkInfo,
        subscribe::{Facility, InterestMaskSet},
    },
    mainloop::threaded::Mainloop,
    volume::{ChannelVolumes, Volume},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::{
    device::{Device, DeviceId},
    event::CoreEvent,
    protocol::{PacketType, ProtocolPacket},
};

const MAX_VOLUME: u32 = 0x10000;

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinkInfo {
    pub name: String,
    pub description: String,
    pub muted: bool,
    pub volume: u32,
    #[serde(rename = "maxVolume")]
    pub max_volume: u32,
    pub enabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemVolume {
    #[serde(rename = "sinkList")]
    pub sink_list: Vec<SinkInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemVolumeRequest {
    #[serde(rename = "requestSinks")]
    pub request_sinks: Option<bool>,
    pub name: Option<String>,
    pub volume: Option<u32>,
    pub muted: Option<bool>,
    pub enabled: Option<bool>,
}

// ---------------------------------------------------------------------------
// Internal PA thread messages
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)]
enum PaMessage {
    GetSinks,
    SetSinkVolume(String, u32),
    SetSinkMute(String, bool),
    Quit,
}

// ---------------------------------------------------------------------------
// Connection wrapper
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct PaConnection {
    tx: mpsc::UnboundedSender<PaMessage>,
}

impl PaConnection {
    fn send(&self, msg: PaMessage) {
        let _ = self.tx.send(msg);
    }
}

// Global map: device_id → PA connection handle.
static PA_CONNECTIONS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, PaConnection>>> =
    std::sync::LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl SystemVolumeRequest {
    pub async fn handle(&self, device: &Device, _core_tx: mpsc::UnboundedSender<CoreEvent>) {
        info!("[systemvolume] handle called: request_sinks={:?} name={:?} volume={:?} muted={:?}",
            self.request_sinks, self.name, self.volume, self.muted);
        let conn = {
            let map = PA_CONNECTIONS.lock().unwrap();
            map.get(&device.device_id.0).cloned()
        };

        let Some(conn) = conn else {
            warn!("[systemvolume] no PA connection for {}", device.device_id);
            return;
        };

        if self.request_sinks == Some(true) {
            debug!("[systemvolume] phone requested sink list");
            conn.send(PaMessage::GetSinks);
            return;
        }

        if let Some(name) = self.name.clone() {
            if let Some(vol) = self.volume {
                debug!("[systemvolume] set volume {} on '{}'", vol, name);
                conn.send(PaMessage::SetSinkVolume(name.clone(), vol));
            }
            if let Some(muted) = self.muted {
                debug!("[systemvolume] set mute {} on '{}'", muted, name);
                conn.send(PaMessage::SetSinkMute(name, muted));
            }
        }
    }
}

/// Called on device connect — starts the persistent PA thread for this device.
pub fn on_device_connect(device_id: DeviceId, core_tx: mpsc::UnboundedSender<CoreEvent>) {
    info!("[systemvolume] on_device_connect for {}", device_id.0);

    let (tx, rx) = mpsc::unbounded_channel::<PaMessage>();
    let conn = PaConnection { tx };

    {
        let mut map = PA_CONNECTIONS.lock().unwrap();
        // Drop the old sender — this closes the old thread's channel, causing it to exit cleanly.
        map.remove(&device_id.0);
        map.insert(device_id.0.clone(), conn);
    }

    let did = device_id.clone();
    std::thread::spawn(move || {
        pa_thread(did.clone(), core_tx, rx);
        let mut map = PA_CONNECTIONS.lock().unwrap();
        map.remove(&did.0);
        info!("[systemvolume] PA thread exited for {}", did.0);
    });
}

// ---------------------------------------------------------------------------
// PA thread — persistent connection with exponential backoff retry
// ---------------------------------------------------------------------------

fn pa_thread(
    device_id: DeviceId,
    core_tx: mpsc::UnboundedSender<CoreEvent>,
    mut rx: mpsc::UnboundedReceiver<PaMessage>,
) {
    let mut retry_count: u32 = 0;

    'outer: loop {
        if retry_count > 0 {
            let delay_ms = (100u64 * (1u64 << (retry_count - 1).min(6))).min(5000);
            info!(
                "[systemvolume] PA reconnect in {}ms (attempt {})",
                delay_ms, retry_count
            );
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        let mut mainloop = match Mainloop::new() {
            Some(m) => m,
            None => {
                warn!("[systemvolume] PA mainloop create failed");
                retry_count += 1;
                continue;
            }
        };

        let mut context = match Context::new(&mainloop, "kdeconnect-systemvolume") {
            Some(c) => c,
            None => {
                warn!("[systemvolume] PA context create failed");
                retry_count += 1;
                continue;
            }
        };

        mainloop.lock();

        let server = std::env::var("PULSE_SERVER").ok();
        let server_ref = server.as_deref();
        info!("[systemvolume] connecting to: {:?}", server_ref);
        if context
            .connect(server_ref, ContextFlagSet::NOFLAGS, None)
            .is_err()
        {
            warn!("[systemvolume] PA context.connect() failed");
            mainloop.unlock();
            retry_count += 1;
            continue;
        }

        if mainloop.start().is_err() {
            warn!("[systemvolume] PA mainloop start failed");
            mainloop.unlock();
            retry_count += 1;
            continue;
        }

        info!("[systemvolume] connecting to PulseAudio...");

        // Wait for context ready with timeout.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let ready = loop {
            match context.get_state() {
                ContextState::Ready => break true,
                ContextState::Failed | ContextState::Terminated => break false,
                _ => {
                    if std::time::Instant::now() > deadline {
                        warn!("[systemvolume] PA connect timed out after 5s");
                        break false;
                    }
                    // Use timed wait instead of blocking wait
                    mainloop.unlock();
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    mainloop.lock();
                }
            }
        };

        if !ready {
            mainloop.unlock();
            context.disconnect();
            mainloop.stop();
            retry_count += 1;
            continue;
        }

        info!("[systemvolume] PA connected");
        retry_count = 0;

        // Subscribe to sink changes — signal ourselves via the message channel.
        let did_sub = device_id.clone();
        context.set_subscribe_callback(Some(Box::new(move |facility, _op, _index| {
            if facility == Some(Facility::Sink) {
                let map = PA_CONNECTIONS.lock().unwrap();
                if let Some(conn) = map.get(&did_sub.0) {
                    conn.send(PaMessage::GetSinks);
                }
            }
        })));
        context.subscribe(InterestMaskSet::SINK, |_| {});

        // Send initial sink list.
        mainloop.unlock();
        if let Some(sinks) = enumerate_sinks(&mut mainloop, &context) {
            info!(
                "[systemvolume] sending {} initial sink(s) to phone",
                sinks.len()
            );
            send_sink_list(sinks, &device_id, &core_tx);
        }

        // Message loop.
        loop {
            // Check context is still alive.
            mainloop.lock();
            let state = context.get_state();
            mainloop.unlock();

            match state {
                ContextState::Ready => {}
                _ => {
                    warn!("[systemvolume] PA context lost — reconnecting");
                    context.disconnect();
                    mainloop.stop();
                    retry_count += 1;
                    continue 'outer;
                }
            }

            match rx.try_recv() {
                Ok(PaMessage::Quit) => {
                    info!("[systemvolume] PA thread quitting for {}", device_id.0);
                    context.disconnect();
                    mainloop.stop();
                    return;
                }
                Ok(PaMessage::GetSinks) => {
                    // Drain duplicate GetSinks — only respond once.
                    while matches!(rx.try_recv(), Ok(PaMessage::GetSinks)) {}
                    if let Some(sinks) = enumerate_sinks(&mut mainloop, &context) {
                        send_sink_list(sinks, &device_id, &core_tx);
                    }
                }
                Ok(PaMessage::SetSinkVolume(name, volume)) => {
                    // Deduplicate rapid volume changes for the same sink.
                    let mut final_vol = volume;
                    while let Ok(PaMessage::SetSinkVolume(n, v)) = rx.try_recv() {
                        if n == name {
                            final_vol = v;
                        }
                    }
                    info!("[systemvolume] set volume {} on '{}'", final_vol, name);
                    set_volume(&mut mainloop, &context, &name, final_vol);
                }
                Ok(PaMessage::SetSinkMute(name, muted)) => {
                    info!("[systemvolume] set mute {} on '{}'", muted, name);
                    set_mute(&mut mainloop, &context, &name, muted);
                }
                Err(mpsc::error::TryRecvError::Empty) => {
                    // Nothing pending — yield briefly.
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    info!(
                        "[systemvolume] channel closed for {} — exiting",
                        device_id.0
                    );
                    context.disconnect();
                    mainloop.stop();
                    return;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PA operations
// ---------------------------------------------------------------------------

fn enumerate_sinks(mainloop: &mut Mainloop, context: &Context) -> Option<Vec<SinkInfo>> {
    // First get the default sink name.
    let default_sink: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let default_sink_cb = Arc::clone(&default_sink);

    mainloop.lock();
    let introspect = context.introspect();
    let op = introspect.get_server_info(move |info| {
        *default_sink_cb.lock().unwrap() = info.default_sink_name
            .as_deref()
            .map(|s| s.to_string());
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while op.get_state() == libpulse_binding::operation::State::Running {
        if std::time::Instant::now() > deadline {
            warn!("[systemvolume] get_server_info timed out");
            break;
        }
        mainloop.unlock();
        std::thread::sleep(std::time::Duration::from_millis(50));
        mainloop.lock();
    }
    mainloop.unlock();

    let default_name = default_sink.lock().unwrap().clone();
    info!("[systemvolume] default sink: {:?}", default_name);

    // Now enumerate sinks.
    let sinks: Arc<Mutex<Vec<SinkInfo>>> = Arc::new(Mutex::new(Vec::new()));
    let sinks_cb = Arc::clone(&sinks);
    let default_name_cb = default_name.clone();

    mainloop.lock();
    let op = introspect.get_sink_info_list(move |result| {
        if let ListResult::Item(info) = result {
            if let Some(sink) = pa_sink_to_info(info, &default_name_cb) {
                sinks_cb.lock().unwrap().push(sink);
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while op.get_state() == libpulse_binding::operation::State::Running {
        if std::time::Instant::now() > deadline {
            warn!("[systemvolume] enumerate_sinks timed out");
            mainloop.unlock();
            return None;
        }
        mainloop.unlock();
        std::thread::sleep(std::time::Duration::from_millis(50));
        mainloop.lock();
    }
    mainloop.unlock();

    Arc::try_unwrap(sinks).ok()?.into_inner().ok()
}

fn set_volume(mainloop: &mut Mainloop, context: &Context, name: &str, volume: u32) {
    let channels: Arc<Mutex<u8>> = Arc::new(Mutex::new(2));
    let channels_cb = Arc::clone(&channels);

    mainloop.lock();
    let mut introspect = context.introspect();
    let op = introspect.get_sink_info_by_name(name, move |result| {
        if let ListResult::Item(info) = result {
            *channels_cb.lock().unwrap() = info.channel_map.len() as u8;
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while op.get_state() == libpulse_binding::operation::State::Running {
        if std::time::Instant::now() > deadline { break; }
        mainloop.unlock();
        std::thread::sleep(std::time::Duration::from_millis(50));
        mainloop.lock();
    }

    let ch = *channels.lock().unwrap();
    let mut cvol = ChannelVolumes::default();
    cvol.set(ch, Volume(volume));

    let op = introspect.set_sink_volume_by_name(name, &cvol, None::<Box<dyn FnMut(bool)>>);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while op.get_state() == libpulse_binding::operation::State::Running {
        if std::time::Instant::now() > deadline { break; }
        mainloop.unlock();
        std::thread::sleep(std::time::Duration::from_millis(50));
        mainloop.lock();
    }
    mainloop.unlock();
}

fn set_mute(mainloop: &mut Mainloop, context: &Context, name: &str, muted: bool) {
    mainloop.lock();
    let mut introspect = context.introspect();
    let op = introspect.set_sink_mute_by_name(name, muted, None::<Box<dyn FnMut(bool)>>);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while op.get_state() == libpulse_binding::operation::State::Running {
        if std::time::Instant::now() > deadline { break; }
        mainloop.unlock();
        std::thread::sleep(std::time::Duration::from_millis(50));
        mainloop.lock();
    }
    mainloop.unlock();
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn pa_sink_to_info(info: &PASinkInfo, default_sink: &Option<String>) -> Option<SinkInfo> {
    let name = info.name.as_deref()?.to_string();
    let description = info.description.as_deref().unwrap_or(&name).to_string();
    let enabled = default_sink.as_deref() == Some(name.as_str());
    info!("[systemvolume] sink '{}' volume={} max={} enabled={}", name, info.volume.avg().0, MAX_VOLUME, enabled);
    Some(SinkInfo {
        name,
        description,
        muted: info.mute,
        volume: info.volume.avg().0,
        max_volume: MAX_VOLUME,
        enabled,
    })
}

fn send_sink_list(
    sinks: Vec<SinkInfo>,
    device_id: &DeviceId,
    core_tx: &mpsc::UnboundedSender<CoreEvent>,
) {
    let packet = ProtocolPacket::new(
        PacketType::SystemVolume,
        serde_json::to_value(SystemVolume { sink_list: sinks }).unwrap(),
    );
    let _ = core_tx.send(CoreEvent::SendPacket {
        device: device_id.clone(),
        packet,
    });
}
