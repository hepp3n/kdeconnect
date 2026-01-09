use serde::{Deserialize, Serialize};

use crate::plugin_interface::Plugin;

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct KeyboardState {
    state: Option<bool>,
}

impl Plugin for KeyboardState {
    fn id(&self) -> &'static str {
        "kdeconnect.mousepad.keyboardstate"
    }
}
