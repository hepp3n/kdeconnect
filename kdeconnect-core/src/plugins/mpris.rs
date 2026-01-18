use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::debug;

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
    // undocumented kdeconnect-kde field
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MprisRequest {
    pub player: Option<String>,
    #[serde(rename = "requestNowPlaying")]
    pub request_now_playing: Option<bool>,
    #[serde(rename = "requestPlayerList")]
    pub request_player_list: Option<bool>,
    #[serde(rename = "requestVolume")]
    pub request_volume: Option<bool>,
    #[serde(rename = "Seek")]
    pub seek: Option<i64>,
    #[serde(rename = "setLoopStatus")]
    pub set_loop_status: Option<MprisLoopStatus>,
    #[serde(rename = "SetPosition")]
    pub set_position: Option<i64>,
    #[serde(rename = "setShuffle")]
    pub set_shuffle: Option<bool>,
    #[serde(rename = "setVolume")]
    pub set_volume: Option<i64>,
    pub action: Option<MprisAction>,
    #[serde(rename = "albumArtUrl")]
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
            // Actions scope - keeps `player` (!Send) from living across .await points later
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

            // Album Art Request
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

            // Info Request
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
