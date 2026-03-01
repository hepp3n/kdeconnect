use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::debug;
use zbus::interface;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

use crate::{
    device::{Device, DeviceManager},
    plugin_interface::Plugin,
    protocol::{DeviceFile, DevicePayload, PacketPayloadTransferInfo, PacketType, ProtocolPacket},
};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Mpris {
    List {
        #[serde(rename = "playerList")]
        player_list: Vec<String>,
        #[serde(rename = "supportAlbumArtPayload")]
        supports_album_art_payload: bool,
    },
    TransferringArt {
        player: String,
        #[serde(rename = "albumArtUrl")]
        album_art_url: String,
        #[serde(rename = "transferringAlbumArt")]
        transferring_album_art: bool,
    },
    Info(MprisPlayer),
}

impl Default for Mpris {
    fn default() -> Self {
        Self::Info(MprisPlayer::new(None).expect("should not panic"))
    }
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct MprisPlayer {
    pub player: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    #[serde(rename = "isPlaying")]
    pub is_playing: Option<bool>,
    #[serde(rename = "canPause")]
    pub can_pause: Option<bool>,
    #[serde(rename = "canPlay")]
    pub can_play: Option<bool>,
    #[serde(rename = "canGoNext")]
    pub can_go_next: Option<bool>,
    #[serde(rename = "canGoPrevious")]
    pub can_go_previous: Option<bool>,
    #[serde(rename = "canSeek")]
    pub can_seek: Option<bool>,
    #[serde(rename = "loopStatus")]
    pub loop_status: Option<MprisLoopStatus>,
    pub shuffle: Option<bool>,
    pub pos: Option<i32>,
    pub length: Option<i32>,
    pub volume: Option<i32>,
    #[serde(rename = "albumArtUrl")]
    pub album_art_url: Option<String>,
    pub url: Option<String>,
}

impl MprisPlayer {
    pub fn new(player_name: Option<&str>) -> anyhow::Result<Self> {
        if player_name.is_none() {
            Ok(Self::default())
        } else {
            let player = get_mpris_players(player_name)?;
            let metadata = get_mpris_metadata(player_name)?;

            let artist = metadata
                .artists()
                .map_or("Unknown Artist".to_string(), |a| a.join(", "));
            let title = metadata
                .title()
                .map_or("Unknown Title".to_string(), |t| t.to_string());
            let album = metadata
                .album_name()
                .map_or("Unknown Album".to_string(), |al| al.to_string());

            let album_art_url = metadata
                .art_url()
                .map_or("".to_string(), |url| url.to_string());

            let length = metadata.length().map_or(0, |l| l.as_millis() as i32);
            let volume = (player.get_volume().unwrap_or(1.0) * 100.0) as i32;
            let can_pause = player.can_pause().unwrap_or(false);
            let can_play = player.can_play().unwrap_or(false);
            let can_go_next = player.can_go_next().unwrap_or(false);
            let can_go_previous = player.can_go_previous().unwrap_or(false);
            let can_seek = player.can_seek().unwrap_or(false);
            let is_playing = player.is_running();
            let loop_status =
                MprisLoopStatus::from(player.get_loop_status().unwrap_or(mpris::LoopStatus::None));
            let shuffle = player.get_shuffle().unwrap_or(false);
            let pos = player.get_position().map_or(0, |p| p.as_millis() as i32);

            let player_ident = player.identity().to_string();

            Ok(Self {
                player: player_ident.clone(),
                title: Some(title),
                artist: Some(artist),
                album: Some(album),
                is_playing: Some(is_playing),
                can_pause: Some(can_pause),
                can_play: Some(can_play),
                can_go_next: Some(can_go_next),
                can_go_previous: Some(can_go_previous),
                can_seek: Some(can_seek),
                loop_status: Some(loop_status),
                shuffle: Some(shuffle),
                pos: Some(pos),
                length: Some(length),
                volume: Some(volume),
                album_art_url: Some(album_art_url.clone()),
                url: None,
            })
        }
    }
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum MprisLoopStatus {
    None,
    Track,
    Playlist,
}

impl From<mpris::LoopStatus> for MprisLoopStatus {
    fn from(loop_status: mpris::LoopStatus) -> Self {
        match loop_status {
            mpris::LoopStatus::None => MprisLoopStatus::None,
            mpris::LoopStatus::Track => MprisLoopStatus::Track,
            mpris::LoopStatus::Playlist => MprisLoopStatus::Playlist,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MprisRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub player: Option<String>,
    #[serde(rename = "requestNowPlaying", skip_serializing_if = "Option::is_none")]
    pub request_now_playing: Option<bool>,
    #[serde(rename = "requestPlayerList", skip_serializing_if = "Option::is_none")]
    pub request_player_list: Option<bool>,
    #[serde(rename = "requestVolume", skip_serializing_if = "Option::is_none")]
    pub request_volume: Option<bool>,
    #[serde(rename = "Seek", skip_serializing_if = "Option::is_none")]
    pub seek: Option<i64>,
    #[serde(rename = "setLoopStatus", skip_serializing_if = "Option::is_none")]
    pub set_loop_status: Option<MprisLoopStatus>,
    #[serde(rename = "SetPosition", skip_serializing_if = "Option::is_none")]
    pub set_position: Option<i64>,
    #[serde(rename = "setShuffle", skip_serializing_if = "Option::is_none")]
    pub set_shuffle: Option<bool>,
    #[serde(rename = "setVolume", skip_serializing_if = "Option::is_none")]
    pub set_volume: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<MprisAction>,
    #[serde(rename = "albumArtUrl", skip_serializing_if = "Option::is_none")]
    pub album_art_url: Option<String>,
}

#[derive(Serialize, Deserialize, Copy, Clone, Debug)]
pub enum MprisAction {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
}

impl Plugin for Mpris {
    fn id(&self) -> &'static str {
        "kdeconnect.mpris"
    }
}

impl Mpris {
    pub async fn send_art(
        &self,
        writer: &mpsc::UnboundedSender<ProtocolPacket>,
        payload_size: i64,
        payload_transfer_info: Option<PacketPayloadTransferInfo>,
    ) {
        let packet = ProtocolPacket::new_with_payload(
            PacketType::Mpris,
            serde_json::to_value(self.clone()).expect("failed serialize packet body"),
            payload_size,
            payload_transfer_info,
        );

        let _ = writer.send(packet);
    }
}

pub fn get_mpris_players(name: Option<&str>) -> anyhow::Result<mpris::Player> {
    let player_finder = mpris::PlayerFinder::new()?;
    if let Some(name) = name {
        Ok(player_finder.find_by_name(name)?)
    } else {
        Ok(player_finder.find_active()?)
    }
}

pub fn get_mpris_metadata(name: Option<&str>) -> anyhow::Result<mpris::Metadata> {
    Ok(get_mpris_players(name)?.get_metadata()?)
}

impl Plugin for MprisRequest {
    fn id(&self) -> &'static str {
        "kdeconnect.mpris.request"
    }
}

impl MprisRequest {
    pub async fn received_packet(
        &self,
        device: &Device,
        core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        debug!("mpris request received: {:?}", self);

        if self.request_player_list == Some(true) {
            debug!("MPRIS Player list requested");
            let players = get_mpris_players(None);

            if let Ok(player) = players {
                let packet = ProtocolPacket::new(
                    PacketType::Mpris,
                    serde_json::to_value(Mpris::List {
                        player_list: vec![player.identity().into()],
                        supports_album_art_payload: true,
                    })
                    .unwrap(),
                );

                let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                    device: device.device_id.clone(),
                    packet,
                });
            }
            return;
        }

        if let Some(player_name) = &self.player {
            {
                if let Ok(player) = get_mpris_players(Some(player_name)) {
                    if let Some(seek_us) = self.seek {
                        let _ = player.seek(seek_us);
                    }

                    if let Some(vol) = self.set_volume {
                        let _ = player.set_volume(vol as f64 / 100.0);
                    }

                    if let Some(loop_status) = self.set_loop_status {
                        let status = match loop_status {
                            MprisLoopStatus::None => mpris::LoopStatus::None,
                            MprisLoopStatus::Track => mpris::LoopStatus::Track,
                            MprisLoopStatus::Playlist => mpris::LoopStatus::Playlist,
                        };
                        let _ = player.set_loop_status(status);
                    }

                    if let Some(position_ms) = self.set_position
                        && let Ok(metadata) = player.get_metadata()
                        && let Some(track_id) = metadata.track_id()
                    {
                        let duration = std::time::Duration::from_millis(position_ms as u64);
                        let _ = player.set_position(track_id, &duration);
                    }

                    if let Some(shuffle) = self.set_shuffle {
                        let _ = player.set_shuffle(shuffle);
                    }

                    if let Some(command) = self.action {
                        let _ = match command {
                            MprisAction::Play => player.play(),
                            MprisAction::Pause => player.pause(),
                            MprisAction::PlayPause => player.play_pause(),
                            MprisAction::Stop => player.stop(),
                            MprisAction::Next => player.next(),
                            MprisAction::Previous => player.previous(),
                        };
                    }
                }
            }

            if let Some(album_art_url) = &self.album_art_url {
                let Ok(player_info) = MprisPlayer::new(Some(player_name)) else {
                    return;
                };
                let art = player_info.album_art_url.clone();

                let path = album_art_url.strip_prefix("file://");

                let Some(path) = path else {
                    return;
                };

                if let Ok(file) = DeviceFile::open(path).await {
                    let payload = DevicePayload::from(file);

                    let construct_packet = ProtocolPacket::new_with_payload(
                        PacketType::Mpris,
                        serde_json::to_value(Mpris::TransferringArt {
                            player: player_name.clone(),
                            album_art_url: art.unwrap_or(path.to_string()),
                            transferring_album_art: true,
                        })
                        .unwrap(),
                        payload.size,
                        None,
                    );

                    let _ = core_tx.send(crate::event::CoreEvent::SendPaylod {
                        device: device.device_id.clone(),
                        packet: construct_packet,
                        payload: Box::new(payload.buf),
                        payload_size: payload.size,
                    });
                }
                return;
            }

            if (self.request_now_playing == Some(true) || self.request_volume == Some(true))
                && let Ok(player_info) = MprisPlayer::new(Some(player_name))
            {
                let construct_packet = ProtocolPacket::new(
                    PacketType::Mpris,
                    serde_json::to_value(Mpris::Info(player_info)).unwrap(),
                );

                let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                    device: device.device_id.clone(),
                    packet: construct_packet,
                });
            }
        }
    }

    pub async fn send_packet(
        &self,
        device: &Device,
        core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
    ) {
        let packet = ProtocolPacket::new(
            PacketType::MprisRequest,
            serde_json::to_value(self).unwrap(),
        );

        let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
            device: device.device_id.clone(),
            packet,
        });
    }
}

pub fn monitor_mpris(
    device_manager: DeviceManager,
    core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
) {
    tokio::task::spawn_blocking(move || {
        let finder = match mpris::PlayerFinder::new() {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Failed to create PlayerFinder: {}", e);
                return;
            }
        };

        loop {
            let player = match finder.find_active() {
                Ok(p) => p,
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                }
            };

            let events = match player.events() {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("Failed to get player events: {}", e);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                }
            };

            for event in events {
                match event {
                    Ok(mpris::Event::Playing)
                    | Ok(mpris::Event::Paused)
                    | Ok(mpris::Event::Stopped)
                    | Ok(mpris::Event::TrackChanged(_))
                    | Ok(mpris::Event::Seeked { .. })
                    | Ok(mpris::Event::VolumeChanged(_)) => {
                        let identity = player.identity();
                        if let Ok(mpris_player) = MprisPlayer::new(Some(identity)) {
                            let packet = ProtocolPacket::new(
                                PacketType::Mpris,
                                serde_json::to_value(Mpris::Info(mpris_player)).unwrap(),
                            );

                            let dm = device_manager.clone();
                            let ctx = core_tx.clone();

                            tokio::spawn(async move {
                                let devices = dm.get_devices().await;
                                for device in devices {
                                    let _ = ctx.send(crate::event::CoreEvent::SendPacket {
                                        device: device.device_id.clone(),
                                        packet: packet.clone(),
                                    });
                                }
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("MPRIS event error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        }
    });
}

// ============================================================================
// D-Bus MPRIS Proxy - Exposes phone media players to desktop media controls
// ============================================================================

struct PhoneMprisPlayer {
    device_id: crate::device::DeviceId,
    player_state: Arc<RwLock<MprisPlayer>>,
    core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl PhoneMprisPlayer {
    async fn play(&self) {
        let mut state = self.player_state.write().await;
        let is_playing = state.is_playing.unwrap_or(false);
        let action = if is_playing {
            MprisAction::Pause
        } else {
            MprisAction::Play
        };
        
        // Optimistically update state immediately
        state.is_playing = Some(!is_playing);
        
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(action),
            ..Default::default()
        };
        drop(state); // Release lock before async call
        
        self.send_request(request).await;
    }

    async fn pause(&self) {
        let mut state = self.player_state.write().await;
        state.is_playing = Some(false);
        
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(MprisAction::Pause),
            ..Default::default()
        };
        drop(state);
        
        self.send_request(request).await;
    }

    async fn play_pause(&self) {
        let mut state = self.player_state.write().await;
        let is_playing = state.is_playing.unwrap_or(false);
        state.is_playing = Some(!is_playing);
        
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(MprisAction::PlayPause),
            ..Default::default()
        };
        drop(state);
        
        self.send_request(request).await;
    }

    async fn next(&self) {
        let state = self.player_state.read().await;
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(MprisAction::Next),
            ..Default::default()
        };
        
        self.send_request(request).await;
    }

    async fn previous(&self) {
        let state = self.player_state.read().await;
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(MprisAction::Previous),
            ..Default::default()
        };
        
        self.send_request(request).await;
    }

    async fn stop(&self) {
        let state = self.player_state.read().await;
        let request = MprisRequest {
            player: Some(state.player.clone()),
            action: Some(MprisAction::Stop),
            ..Default::default()
        };
        
        self.send_request(request).await;
    }

    #[zbus(property)]
    async fn playback_status(&self) -> String {
        let state = self.player_state.read().await;
        if state.is_playing.unwrap_or(false) {
            "Playing".to_string()
        } else {
            "Paused".to_string()
        }
    }

    #[zbus(property)]
    async fn metadata(&self) -> HashMap<String, zbus::zvariant::Value<'static>> {
        let state = self.player_state.read().await;
        let mut metadata = HashMap::new();
        
        if let Some(ref title) = state.title {
            metadata.insert("xesam:title".to_string(), 
                zbus::zvariant::Value::new(title.clone()));
        }
        if let Some(ref artist) = state.artist {
            metadata.insert("xesam:artist".to_string(), 
                zbus::zvariant::Value::new(vec![artist.clone()]));
        }
        if let Some(ref album) = state.album {
            metadata.insert("xesam:album".to_string(), 
                zbus::zvariant::Value::new(album.clone()));
        }
        if let Some(length) = state.length {
            metadata.insert("mpris:length".to_string(), 
                zbus::zvariant::Value::new(length as i64 * 1000));
        }
        
        metadata
    }

    #[zbus(property)]
    async fn can_play(&self) -> bool {
        let state = self.player_state.read().await;
        state.can_play.unwrap_or(true)
    }

    #[zbus(property)]
    async fn can_pause(&self) -> bool {
        let state = self.player_state.read().await;
        state.can_pause.unwrap_or(true)
    }

    #[zbus(property)]
    async fn can_go_next(&self) -> bool {
        let state = self.player_state.read().await;
        state.can_go_next.unwrap_or(true)
    }

    #[zbus(property)]
    async fn can_go_previous(&self) -> bool {
        let state = self.player_state.read().await;
        state.can_go_previous.unwrap_or(true)
    }

    #[zbus(property)]
    async fn volume(&self) -> f64 {
        let state = self.player_state.read().await;
        state.volume.unwrap_or(50) as f64 / 100.0
    }

    #[zbus(property)]
    async fn position(&self) -> i64 {
        let state = self.player_state.read().await;
        state.pos.unwrap_or(0) as i64 * 1000
    }
}

impl PhoneMprisPlayer {
    async fn send_request(&self, request: MprisRequest) {
        let packet = ProtocolPacket::new(
            PacketType::MprisRequest,
            serde_json::to_value(request).unwrap(),
        );
        
        let _ = self.core_tx.send(crate::event::CoreEvent::SendPacket {
            device: self.device_id.clone(),
            packet,
        });
        
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let player_name = self.player_state.read().await.player.clone();
        let info_request = MprisRequest {
            player: Some(player_name),
            request_now_playing: Some(true),
            ..Default::default()
        };
        let info_packet = ProtocolPacket::new(
            PacketType::MprisRequest,
            serde_json::to_value(info_request).unwrap(),
        );
        let _ = self.core_tx.send(crate::event::CoreEvent::SendPacket {
            device: self.device_id.clone(),
            packet: info_packet,
        });
    }
}

struct PhoneMprisRoot {
    device_name: String,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl PhoneMprisRoot {
    async fn raise(&self) {
    }

    async fn quit(&self) {
    }

    #[zbus(property)]
    async fn identity(&self) -> String {
        format!("KDE Connect - {}", self.device_name)
    }

    #[zbus(property)]
    async fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    async fn has_track_list(&self) -> bool {
        false
    }
}

struct PhonePlayerConnection {
    connection: zbus::Connection,
    player_state: Arc<RwLock<MprisPlayer>>,
}

// Replace the expose_phone_mpris function with this corrected version:

pub fn expose_phone_mpris(
    mut conn_rx: mpsc::UnboundedReceiver<crate::event::ConnectionEvent>,
    core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
) {
    tokio::spawn(async move {
        let mut active_players: HashMap<String, PhonePlayerConnection> = HashMap::new();
        let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(500));

        loop {
            tokio::select! {
                Some(event) = conn_rx.recv() => {
                    match event {
                        crate::event::ConnectionEvent::Mpris((device_id, mpris_data)) => {
                            match mpris_data {
                                Mpris::Info(player_info) => {
                                    let player_key = format!("{}_{}", device_id.0, player_info.player);
                                    
                                    if let Some(player_conn) = active_players.get_mut(&player_key) {
                                        let old_state = player_conn.player_state.read().await.clone();
                                        
                                        {
                                            let mut state = player_conn.player_state.write().await;
                                            *state = player_info.clone();
                                        }
                                        
                                        if old_state.is_playing != player_info.is_playing {
                                            let obj_server = player_conn.connection.object_server();
                                            if let Ok(iface_ref) = obj_server.interface::<_, PhoneMprisPlayer>("/org/mpris/MediaPlayer2").await {
                                                let emitter = iface_ref.signal_emitter();
                                                let iface = iface_ref.get().await;
                                                let _ = iface.playback_status_changed(&emitter).await;
                                            }
                                        }
                                        
                                        tracing::debug!("Updated phone MPRIS player: {}", player_key);
                                    } else {
                                        match register_phone_player(&device_id, &player_info, core_tx.clone()).await {
                                            Ok((conn, state)) => {
                                                tracing::info!("✓ Registered D-Bus MPRIS for phone player: {}", player_key);
                                                active_players.insert(player_key.clone(), PhonePlayerConnection {
                                                    connection: conn,
                                                    player_state: state,
                                                });
                                            }
                                            Err(e) => {
                                                tracing::error!("✗ Failed to register D-Bus MPRIS: {}", e);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        crate::event::ConnectionEvent::Disconnected(device_id) => {
                            active_players.retain(|key, _| !key.starts_with(&device_id.0));
                            tracing::info!("Removed MPRIS players for disconnected device: {}", device_id.0);
                        }
                        _ => {}
                    }
                }
                _ = interval.tick() => {
                    for (player_key, player_conn) in &active_players {
                        if let Some((device_id_str, _)) = player_key.split_once('_') {
                            let player_name = player_conn.player_state.read().await.player.clone();
                            let info_request = MprisRequest {
                                player: Some(player_name),
                                request_now_playing: Some(true),
                                ..Default::default()
                            };
                            let packet = ProtocolPacket::new(
                                PacketType::MprisRequest,
                                serde_json::to_value(info_request).unwrap(),
                            );
                            let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                                device: crate::device::DeviceId(device_id_str.to_string()),
                                packet,
                            });
                        }
                    }
                }
            }
        }
    });
}

async fn register_phone_player(
    device_id: &crate::device::DeviceId,
    player_info: &MprisPlayer,
    core_tx: mpsc::UnboundedSender<crate::event::CoreEvent>,
) -> anyhow::Result<(zbus::Connection, Arc<RwLock<MprisPlayer>>)> {
    let service_name = format!("org.mpris.MediaPlayer2.KDEConnect_{}", 
        device_id.0.replace("-", "_"));
    
    let player_state = Arc::new(RwLock::new(player_info.clone()));
    
    let phone_player = PhoneMprisPlayer {
        device_id: device_id.clone(),
        player_state: player_state.clone(),
        core_tx: core_tx.clone(),
    };
    
    let phone_root = PhoneMprisRoot {
        device_name: device_id.0.clone(),
    };
    
    let conn = zbus::connection::Builder::session()?
        .name(service_name)?
        .serve_at("/org/mpris/MediaPlayer2", phone_player)?
        .serve_at("/org/mpris/MediaPlayer2", phone_root)?
        .build()
        .await?;
    
    Ok((conn, player_state))
}
