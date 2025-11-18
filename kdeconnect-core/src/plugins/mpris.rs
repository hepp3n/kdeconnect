use serde::{Deserialize, Serialize};

use crate::plugin_interface::Plugin;

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
