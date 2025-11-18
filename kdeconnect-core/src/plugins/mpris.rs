use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};
use tracing::debug;

use crate::{
    device::Device,
    plugin_interface::Plugin,
    protocol::{PacketType, ProtocolPacket},
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

#[derive(Serialize, Deserialize, Clone, Debug)]
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
#[serde(untagged)]
pub enum MprisRequest {
    List {
        #[serde(rename = "requestPlayerList")]
        request_player_list: bool,
    },
    PlayerRequest {
        player: String,
        #[serde(rename = "requestNowPlaying")]
        request_now_playing: Option<bool>,
        #[serde(rename = "requestVolume")]
        request_volume: Option<bool>,
        // set to a file:// string to get kdeconnect-kde to send (local) album art
        #[serde(rename = "albumArtUrl", skip_serializing_if = "Option::is_none")]
        request_album_art: Option<String>,
    },
    Action(MprisRequestAction),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct MprisRequestAction {
    pub player: String,
    #[serde(rename = "Seek", skip_serializing_if = "Option::is_none")]
    pub seek: Option<i64>,
    #[serde(rename = "setVolume", skip_serializing_if = "Option::is_none")]
    pub set_volume: Option<i64>,
    #[serde(rename = "setLoopStatus", skip_serializing_if = "Option::is_none")]
    pub set_loop_status: Option<MprisLoopStatus>,
    #[serde(rename = "SetPosition", skip_serializing_if = "Option::is_none")]
    pub set_position: Option<i64>,
    #[serde(rename = "setShuffle", skip_serializing_if = "Option::is_none")]
    pub set_shuffle: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<MprisAction>,
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

#[async_trait::async_trait]
impl Plugin for Mpris {
    fn id(&self) -> &'static str {
        "kdeconnect.mpris"
    }

    async fn received(
        &self,
        _device: &Device,
        _event: Arc<mpsc::UnboundedSender<crate::event::ConnectionEvent>>,
        _core_event: Arc<broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        // MPRIS plugin does not send events on its own
    }

    async fn send(
        &self,
        _device: &Device,
        _core_event: Arc<broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        // MPRIS plugin does not send events on its own
    }
}

pub fn get_mpris_players() -> anyhow::Result<mpris::Player> {
    let player_finder = mpris::PlayerFinder::new()?;
    Ok(player_finder.find_active()?)
}

pub fn get_mpris_metadata() -> Option<mpris::Metadata> {
    if let Ok(player) = get_mpris_players() {
        Some(
            player
                .get_metadata()
                .expect("Failed to get player metadata"),
        )
    } else {
        None
    }
}

#[async_trait::async_trait]
impl Plugin for MprisRequest {
    fn id(&self) -> &'static str {
        "kdeconnect.mpris.request"
    }

    async fn received(
        &self,
        device: &Device,
        _event: Arc<mpsc::UnboundedSender<crate::event::ConnectionEvent>>,
        core_tx: Arc<broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        match self {
            MprisRequest::List {
                request_player_list,
            } => {
                if *request_player_list {
                    debug!("MPRIS Player list requested");
                    let players = get_mpris_players();

                    if let Ok(player) = players {
                        let packet = ProtocolPacket {
                            id: None,
                            packet_type: PacketType::Mpris,
                            body: serde_json::to_value(Mpris::List {
                                player_list: vec![player.identity().into()],
                                supports_album_art_payload: false,
                            })
                            .unwrap(),
                            payload_size: None,
                            payload_transfer_info: None,
                        };

                        let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                            device: device.device_id.clone(),
                            packet,
                        });
                    }
                }
            }
            MprisRequest::PlayerRequest {
                player,
                request_now_playing,
                request_volume,
                request_album_art,
            } => {
                debug!(
                    "mpris request received: {:?} {:?} {:?} {:?} ",
                    player, request_now_playing, request_volume, request_album_art
                );
                let Some(metadata) = get_mpris_metadata() else {
                    return;
                };
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

                let Ok(player) = get_mpris_players() else {
                    return;
                };

                let volume = (player.get_volume().unwrap_or(1.0) * 100.0) as i32;
                let can_pause = player.can_pause().unwrap_or(false);
                let can_play = player.can_play().unwrap_or(false);
                let can_go_next = player.can_go_next().unwrap_or(false);
                let can_go_previous = player.can_go_previous().unwrap_or(false);
                let can_seek = player.can_seek().unwrap_or(false);
                let is_playing = player.is_running();
                let loop_status = MprisLoopStatus::from(
                    player.get_loop_status().unwrap_or(mpris::LoopStatus::None),
                );
                let shuffle = player.get_shuffle().unwrap_or(false);
                let pos = player.get_position().map_or(0, |p| p.as_millis() as i32);

                let construct_packet = ProtocolPacket {
                    id: None,
                    packet_type: PacketType::Mpris,
                    body: serde_json::to_value(Mpris::Info(MprisPlayer {
                        player: player.identity().into(),
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
                        album_art_url: Some(album_art_url),
                        url: None,
                    }))
                    .unwrap(),
                    payload_size: None,
                    payload_transfer_info: None,
                };

                let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                    device: device.device_id.clone(),
                    packet: construct_packet,
                });
            }
            MprisRequest::Action { .. } => {
                debug!("MPRIS Action request received");
            }
        }
    }

    async fn send(
        &self,
        _device: &Device,
        _core_event: Arc<broadcast::Sender<crate::event::CoreEvent>>,
    ) {
        // MPRIS Request plugin does not send events on its own
    }
}
