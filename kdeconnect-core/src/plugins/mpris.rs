use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
    device::Device,
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
        Self::Info(MprisPlayer::new(None).expect("mpris failed"))
    }
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

impl MprisPlayer {
    pub fn new(player_name: Option<&str>) -> anyhow::Result<Self, anyhow::Error> {
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
        match self {
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

                let Ok(player) = MprisPlayer::new(Some(player)) else {
                    return;
                };

                let player_name = player.player.clone();
                let art = player.album_art_url.clone();

                if let Some(album_art_url) = request_album_art {
                    let path = album_art_url.strip_prefix("file://");

                    let Some(path) = path else {
                        return;
                    };

                    if let Ok(file) = DeviceFile::open(path).await {
                        let payload = DevicePayload::from(file);

                        let construct_packet = ProtocolPacket::new_with_payload(
                            PacketType::Mpris,
                            serde_json::to_value(Mpris::TransferringArt {
                                player: player_name,
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
                } else {
                    let construct_packet = ProtocolPacket {
                        id: None,
                        packet_type: PacketType::Mpris,
                        body: serde_json::to_value(Mpris::Info(player)).unwrap(),
                        payload_size: None,
                        payload_transfer_info: None,
                    };

                    let _ = core_tx.send(crate::event::CoreEvent::SendPacket {
                        device: device.device_id.clone(),
                        packet: construct_packet,
                    });
                }
            }
            MprisRequest::List {
                request_player_list,
            } => {
                if *request_player_list {
                    debug!("MPRIS Player list requested");
                    let players = get_mpris_players(None);

                    if let Ok(player) = players {
                        let packet = ProtocolPacket {
                            id: None,
                            packet_type: PacketType::Mpris,
                            body: serde_json::to_value(Mpris::List {
                                player_list: vec![player.identity().into()],
                                supports_album_art_payload: true,
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
            MprisRequest::Action { .. } => {
                debug!("received mpris action");
            }
        }
    }
}
