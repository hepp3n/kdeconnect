use anyhow::anyhow;
use libpulse_binding::{
    callbacks::ListResult,
    context::{
        Context, FlagSet as ContextFlagSet, State as ContextState,
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

/// PA_VOLUME_NORM — matches the protocol's maxVolume field.
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
// Incoming packet handler
// ---------------------------------------------------------------------------

impl SystemVolumeRequest {
    pub async fn handle(&self, device: &Device, core_tx: mpsc::UnboundedSender<CoreEvent>) {
        if self.request_sinks == Some(true) {
            debug!("[systemvolume] sink list requested");
            match tokio::task::spawn_blocking(get_sinks_sync).await {
                Ok(Ok(sinks)) => send_sink_list(sinks, &device.device_id, &core_tx),
                Ok(Err(e)) => warn!("[systemvolume] sink enumeration failed: {}", e),
                Err(e) => warn!("[systemvolume] spawn_blocking join failed: {}", e),
            }
            return;
        }

        if let Some(name) = self.name.clone() {
            if let Some(vol) = self.volume {
                let n = name.clone();
                if let Err(e) =
                    tokio::task::spawn_blocking(move || set_sink_volume_sync(&n, vol)).await
                {
                    warn!("[systemvolume] set_volume join failed: {}", e);
                }
            }
            if let Some(muted) = self.muted {
                if let Err(e) =
                    tokio::task::spawn_blocking(move || set_sink_mute_sync(&name, muted)).await
                {
                    warn!("[systemvolume] set_mute join failed: {}", e);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Device connect entry point
// ---------------------------------------------------------------------------

/// Called on device connect — sends initial sink list then starts the watcher.
pub fn on_device_connect(device_id: DeviceId, core_tx: mpsc::UnboundedSender<CoreEvent>) {
    info!("[systemvolume] on_device_connect called for {}", device_id.0);
    let core_tx_watcher = core_tx.clone();
    let device_id_watcher = device_id.clone();
    tokio::spawn(async move {
        match tokio::task::spawn_blocking(get_sinks_sync).await {
            Ok(Ok(sinks)) => {
                info!("[systemvolume] got {} sink(s), sending to phone", sinks.len());
                send_sink_list(sinks, &device_id, &core_tx);
            }
            Ok(Err(e)) => warn!("[systemvolume] initial sink list failed: {}", e),
            Err(e) => warn!("[systemvolume] spawn_blocking failed: {}", e),
        }
        spawn_volume_watcher(device_id_watcher, core_tx_watcher);
    });
}

// ---------------------------------------------------------------------------
// Volume watcher — pushes updates to phone when desktop volume changes
// ---------------------------------------------------------------------------

pub fn spawn_volume_watcher(device_id: DeviceId, core_tx: mpsc::UnboundedSender<CoreEvent>) {
    // Unbounded channel: PA thread signals "something changed", tokio task responds.
    let (change_tx, mut change_rx) = mpsc::unbounded_channel::<()>();

    // PA subscription runs on a dedicated std thread (libpulse is not async).
    std::thread::spawn(move || {
        if let Err(e) = run_pa_subscription(change_tx) {
            warn!("[systemvolume] PA subscription thread exited: {}", e);
        }
    });

    // Tokio task: on each change signal, enumerate sinks and push to phone.
    tokio::spawn(async move {
        while change_rx.recv().await.is_some() {
            debug!("[systemvolume] sink change received, enumerating");
            match tokio::task::spawn_blocking(get_sinks_sync).await {
                Ok(Ok(sinks)) => send_sink_list(sinks, &device_id, &core_tx),
                Ok(Err(e)) => warn!("[systemvolume] watcher enumeration failed: {}", e),
                Err(e) => warn!("[systemvolume] watcher join failed: {}", e),
            }
        }
    });
}

fn run_pa_subscription(change_tx: mpsc::UnboundedSender<()>) -> anyhow::Result<()> {
    let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("PA mainloop create failed"))?;
    let mut context = Context::new(&mainloop, "kdeconnect-vol-watcher")
        .ok_or_else(|| anyhow!("PA context create failed"))?;

    mainloop.lock();
    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("PA context connect failed"))?;
    mainloop
        .start()
        .map_err(|_| anyhow!("PA mainloop start failed"))?;

    let mut attempts = 0u32;
    loop {
        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                mainloop.unlock();
                return Err(anyhow!("PA context failed to reach ready state"));
            }
            _ => {
                attempts += 1;
                if attempts > 200 {
                    mainloop.unlock();
                    return Err(anyhow!("PA watcher context connect timed out"));
                }
                mainloop.wait();
            }
        }
    }

    context.subscribe(InterestMaskSet::SINK, |_| {});

    let change_tx_cb = change_tx.clone();
    context.set_subscribe_callback(Some(Box::new(move |facility, _op, _index| {
        if facility == Some(Facility::Sink) {
            // Ignore send errors — receiver dropped means device disconnected.
            let _ = change_tx_cb.send(());
        }
    })));

    // Unlock: the PA mainloop thread now runs freely and drives callbacks.
    mainloop.unlock();

    // Keep the thread alive and poll for device disconnect.
    while !change_tx.is_closed() {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    // Cleanup.
    mainloop.lock();
    context.disconnect();
    mainloop.unlock();
    mainloop.stop();

    Ok(())
}

// ---------------------------------------------------------------------------
// libpulse operations (blocking — call via spawn_blocking)
// ---------------------------------------------------------------------------

fn get_sinks_sync() -> anyhow::Result<Vec<SinkInfo>> {
    let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("PA mainloop failed"))?;
    let mut context = Context::new(&mainloop, "kdeconnect-sinks")
        .ok_or_else(|| anyhow!("PA context failed"))?;

    mainloop.lock();
    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("PA connect failed"))?;
    mainloop
        .start()
        .map_err(|_| anyhow!("PA mainloop start failed"))?;

    info!("[systemvolume] connecting to PulseAudio...");
    let mut attempts = 0u32;
    loop {
        match context.get_state() {
            ContextState::Ready => {
                info!("[systemvolume] PA context ready");
                break;
            }
            ContextState::Failed | ContextState::Terminated => {
                mainloop.unlock();
                return Err(anyhow!("PA context failed to connect — is PipeWire/PulseAudio running?"));
            }
            _ => {
                attempts += 1;
                if attempts > 200 {
                    mainloop.unlock();
                    return Err(anyhow!("PA context connect timed out — check PULSE_SERVER socket"));
                }
                mainloop.wait();
            }
        }
    }

    let sinks: Arc<Mutex<Vec<SinkInfo>>> = Arc::new(Mutex::new(Vec::new()));
    let sinks_cb = Arc::clone(&sinks);

    let introspect = context.introspect();
    let op = introspect.get_sink_info_list(move |result| {
        if let ListResult::Item(info) = result {
            let name = info.name.as_deref().unwrap_or("").to_string();
            let description = info.description.as_deref().unwrap_or(&name).to_string();
            sinks_cb.lock().unwrap().push(SinkInfo {
                name,
                description,
                muted: info.mute,
                volume: info.volume.avg().0,
                max_volume: MAX_VOLUME,
                enabled: true,
            });
        }
    });

    while op.get_state() == libpulse_binding::operation::State::Running {
        mainloop.wait();
    }

    mainloop.unlock();
    context.disconnect();
    mainloop.stop();

    Ok(Arc::try_unwrap(sinks).unwrap().into_inner().unwrap())
}

fn set_sink_volume_sync(name: &str, volume: u32) -> anyhow::Result<()> {
    info!("[systemvolume] setting volume on '{}' to {}", name, volume);
    let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("PA mainloop failed"))?;
    let mut context = Context::new(&mainloop, "kdeconnect-setvol")
        .ok_or_else(|| anyhow!("PA context failed"))?;

    mainloop.lock();
    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("PA connect failed"))?;
    mainloop
        .start()
        .map_err(|_| anyhow!("PA mainloop start failed"))?;

    let mut attempts = 0u32;
    loop {
        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                mainloop.unlock();
                return Err(anyhow!("PA context failed"));
            }
            _ => {
                attempts += 1;
                if attempts > 200 {
                    mainloop.unlock();
                    return Err(anyhow!("PA setvol context connect timed out"));
                }
                mainloop.wait();
            }
        }
    }

    // Get the sink's channel count before setting volume.
    let channels: Arc<Mutex<u8>> = Arc::new(Mutex::new(2));
    let channels_cb = Arc::clone(&channels);

    let mut introspect = context.introspect();
    let op = introspect.get_sink_info_by_name(name, move |result| {
        if let ListResult::Item(info) = result {
            *channels_cb.lock().unwrap() = info.channel_map.len() as u8;
        }
    });

    while op.get_state() == libpulse_binding::operation::State::Running {
        mainloop.wait();
    }

    let ch = *channels.lock().unwrap();
    let mut cvol = ChannelVolumes::default();
    cvol.set(ch, Volume(volume));

    let op = introspect.set_sink_volume_by_name(name, &cvol, None::<Box<dyn FnMut(bool)>>);
    while op.get_state() == libpulse_binding::operation::State::Running {
        mainloop.wait();
    }

    mainloop.unlock();
    context.disconnect();
    mainloop.stop();

    Ok(())
}

fn set_sink_mute_sync(name: &str, muted: bool) -> anyhow::Result<()> {
    info!("[systemvolume] setting mute on '{}' to {}", name, muted);
    let mut mainloop = Mainloop::new().ok_or_else(|| anyhow!("PA mainloop failed"))?;
    let mut context = Context::new(&mainloop, "kdeconnect-setmute")
        .ok_or_else(|| anyhow!("PA context failed"))?;

    mainloop.lock();
    context
        .connect(None, ContextFlagSet::NOFLAGS, None)
        .map_err(|_| anyhow!("PA connect failed"))?;
    mainloop
        .start()
        .map_err(|_| anyhow!("PA mainloop start failed"))?;

    let mut attempts = 0u32;
    loop {
        match context.get_state() {
            ContextState::Ready => break,
            ContextState::Failed | ContextState::Terminated => {
                mainloop.unlock();
                return Err(anyhow!("PA context failed"));
            }
            _ => {
                attempts += 1;
                if attempts > 200 {
                    mainloop.unlock();
                    return Err(anyhow!("PA setmute context connect timed out"));
                }
                mainloop.wait();
            }
        }
    }

    let mut introspect = context.introspect();
    let op = introspect.set_sink_mute_by_name(name, muted, None::<Box<dyn FnMut(bool)>>);
    while op.get_state() == libpulse_binding::operation::State::Running {
        mainloop.wait();
    }

    mainloop.unlock();
    context.disconnect();
    mainloop.stop();

    Ok(())
}

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn send_sink_list(sinks: Vec<SinkInfo>, device_id: &DeviceId, core_tx: &mpsc::UnboundedSender<CoreEvent>) {
    let packet = ProtocolPacket::new(
        PacketType::SystemVolume,
        serde_json::to_value(SystemVolume { sink_list: sinks }).unwrap(),
    );
    let _ = core_tx.send(CoreEvent::SendPacket {
        device: device_id.clone(),
        packet,
    });
}
