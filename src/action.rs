use serde::Deserialize;

pub enum Action {
    Recording(RecordingActionProperties),
    Streaming(StreamActionProperties),
    VirtualCamera(VirtualCameraActionProperties),
    SwitchScene(SwitchSceneProperties),
    SwitchProfile(SwitchProfileProperties),
}

impl Action {
    pub fn from_action(
        action_id: &str,
        properties: serde_json::Value,
    ) -> Option<Result<Action, serde_json::Error>> {
        Some(match action_id {
            "recording" => serde_json::from_value(properties).map(Action::Recording),
            "streaming" => serde_json::from_value(properties).map(Action::Streaming),
            "virtual_camera" => serde_json::from_value(properties).map(Action::VirtualCamera),
            "switch_scene" => serde_json::from_value(properties).map(Action::SwitchScene),
            "switch_profile" => serde_json::from_value(properties).map(Action::SwitchProfile),
            _ => return None,
        })
    }
}

#[derive(Deserialize)]
pub struct SwitchSceneProperties {
    pub scene: Option<String>,
}

#[derive(Deserialize)]
pub struct SwitchProfileProperties {
    pub profile: Option<String>,
}

#[derive(Deserialize)]
pub struct RecordingActionProperties {
    pub action: Option<RecordingAction>,
}

#[derive(Deserialize)]
pub enum RecordingAction {
    StartStop,
    Start,
    Stop,
    PauseResume,
    Pause,
    Resume,
}

#[derive(Deserialize)]
pub struct StreamActionProperties {
    pub action: Option<StreamAction>,
}

#[derive(Deserialize)]
pub enum StreamAction {
    StartStop,
    Start,
    Stop,
}

#[derive(Deserialize)]
pub struct VirtualCameraActionProperties {
    pub action: Option<VirtualCameraAction>,
}

#[derive(Deserialize)]
pub enum VirtualCameraAction {
    StartStop,
    Start,
    Stop,
}
