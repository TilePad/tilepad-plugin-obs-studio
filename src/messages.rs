use crate::plugin::Auth;
use serde::{Deserialize, Serialize};

/// Messages from the inspector
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InspectorMessageIn {
    GetClientState,
    GetProfiles,
    GetScenes,
    Connect { auth: Auth },
}

/// Messages to the inspector
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InspectorMessageOut {
    ClientState { state: String },
    Profiles { profiles: Vec<SelectOption> },
    Scenes { scenes: Vec<SelectOption> },
}

/// Option for a select dropdown menu
#[derive(Deserialize, Serialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
}
