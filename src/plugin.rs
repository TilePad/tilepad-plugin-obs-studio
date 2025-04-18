use obws::requests::scenes::SceneId;
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, rc::Rc, str::FromStr};
use tilepad_plugin_sdk::{
    inspector::Inspector,
    plugin::Plugin,
    protocol::TileInteractionContext,
    session::PluginSessionHandle,
    tracing::{self},
};
use tokio::{sync::Mutex, task::spawn_local};
use uuid::Uuid;

/// Properties for the plugin itself
#[derive(Debug, Deserialize, Serialize)]
pub struct Properties {
    pub auth: Option<Auth>,
}

/// Properties for the plugin itself
#[derive(Debug, Deserialize, Serialize)]
pub struct Auth {
    pub host: String,
    pub port: u16,
    pub password: String,
}

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

#[derive(Default)]
pub struct ObsPlugin {
    state: Rc<State>,
}

#[derive(Default)]
pub struct State {
    client_state: Mutex<ClientState>,
    inspector: RefCell<Option<Inspector>>,
}

impl State {
    fn set_inspector(&self, inspector: Option<Inspector>) {
        *self.inspector.borrow_mut() = inspector;
    }

    async fn set_client_state(&self, client_state: ClientState) {
        let state = format!("{}", &client_state);
        {
            *self.client_state.lock().await = client_state;
        }

        if let Some(inspector) = self.inspector.borrow().as_ref() {
            _ = inspector.send(InspectorMessageOut::ClientState { state });
        }
    }

    async fn try_connect(&self, auth: Auth, session: PluginSessionHandle) {
        {
            if matches!(
                &*self.client_state.lock().await,
                ClientState::Connecting | ClientState::Connected { .. }
            ) {
                return;
            }
        }

        // Set to connecting state
        self.set_client_state(ClientState::Connecting).await;

        let mut password: Option<String> = None;
        if !auth.password.trim().is_empty() {
            password = Some(auth.password.clone())
        }

        let client = match obws::Client::connect(auth.host.clone(), auth.port, password).await {
            Ok(value) => value,
            Err(error) => {
                self.set_client_state(ClientState::ConnectError { error })
                    .await;
                return;
            }
        };

        _ = session.set_properties(Properties { auth: Some(auth) });

        self.set_client_state(ClientState::Connected { client })
            .await;
    }
}

#[derive(Default)]
pub enum ClientState {
    /// Initial state
    #[default]
    Initial,

    NotConnected,

    /// Connecting to the client
    Connecting,

    /// Connected to the client
    Connected {
        client: obws::Client,
    },

    /// Connection error
    ConnectError {
        error: obws::error::Error,
    },

    /// Lost connection
    ConnectionLost,
}

impl std::fmt::Display for ClientState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ClientState::Initial => "INITIAL",
            ClientState::NotConnected => "NOT_CONNECTED",
            ClientState::Connecting => "CONNECTING",
            ClientState::Connected { .. } => "CONNECTED",
            ClientState::ConnectError { .. } => "CONNECT_ERROR",
            ClientState::ConnectionLost => "CONNECTION_LOST",
        })
    }
}

impl ObsPlugin {
    pub fn new() -> Self {
        Default::default()
    }
}

impl Plugin for ObsPlugin {
    fn on_properties(&self, session: &PluginSessionHandle, properties: serde_json::Value) {
        let session = session.clone();
        if let Ok(properties) = serde_json::from_value::<Properties>(properties) {
            let auth = match properties.auth {
                Some(value) => value,
                None => {
                    let state = self.state.clone();
                    spawn_local(async move {
                        state.set_client_state(ClientState::NotConnected).await;
                    });
                    return;
                }
            };

            let state = self.state.clone();
            spawn_local(async move {
                state.try_connect(auth, session).await;
            });
        } else {
            let state = self.state.clone();
            spawn_local(async move {
                state.set_client_state(ClientState::NotConnected).await;
            });
        }
    }

    fn on_inspector_open(&self, _session: &PluginSessionHandle, inspector: Inspector) {
        self.state.set_inspector(Some(inspector));
    }

    fn on_inspector_close(&self, _session: &PluginSessionHandle, _inspector: Inspector) {
        self.state.set_inspector(None);
    }

    fn on_inspector_message(
        &self,
        session: &PluginSessionHandle,
        inspector: Inspector,
        message: serde_json::Value,
    ) {
        let message: InspectorMessageIn = match serde_json::from_value(message) {
            Ok(value) => value,
            Err(_) => return,
        };

        let session = session.clone();

        match message {
            InspectorMessageIn::GetClientState => {
                let state = self.state.clone();
                spawn_local(async move {
                    let client_state = &mut *state.client_state.lock().await;
                    let state = { format!("{}", &client_state) };
                    _ = inspector.send(InspectorMessageOut::ClientState { state });
                });
            }
            InspectorMessageIn::Connect { auth } => {
                let state = self.state.clone();
                spawn_local(async move {
                    state.try_connect(auth, session).await;
                });
            }
            InspectorMessageIn::GetProfiles => {
                run_with_client(self.state.clone(), async move |client| {
                    let profiles = client.profiles();
                    let list = match profiles.list().await {
                        Ok(value) => value,
                        Err(cause) => {
                            tracing::error!(?cause, "failed to get profiles");
                            return;
                        }
                    };

                    _ = inspector.send(InspectorMessageOut::Profiles {
                        profiles: list
                            .profiles
                            .into_iter()
                            .map(|profile| SelectOption {
                                label: profile.clone(),
                                value: profile,
                            })
                            .collect(),
                    });
                });
            }
            InspectorMessageIn::GetScenes => {
                run_with_client(self.state.clone(), async move |client| {
                    let scenes = client.scenes();

                    let list = match scenes.list().await {
                        Ok(value) => value,
                        Err(cause) => {
                            tracing::error!(?cause, "failed to get profiles");
                            return;
                        }
                    };

                    _ = inspector.send(InspectorMessageOut::Scenes {
                        scenes: list
                            .scenes
                            .into_iter()
                            .map(|scene| SelectOption {
                                label: scene.id.name,
                                value: scene.id.uuid.to_string(),
                            })
                            .collect(),
                    });
                });
            }
        }
    }

    fn on_tile_clicked(
        &self,
        _session: &PluginSessionHandle,
        ctx: TileInteractionContext,
        properties: serde_json::Value,
    ) {
        let action_id = ctx.action_id.as_str();
        let action = match Action::from_action(action_id, properties) {
            Some(Ok(value)) => value,
            Some(Err(cause)) => {
                tracing::error!(?cause, ?action_id, "failed to deserialize action");
                return;
            }
            None => {
                tracing::debug!(?action_id, "unknown tile action requested");
                return;
            }
        };

        match action {
            Action::Recording(properties) => {
                let action: RecordingAction = match properties.action {
                    Some(value) => value,
                    None => return,
                };

                run_with_client(self.state.clone(), async move |client| match action {
                    RecordingAction::StartStop => {
                        if let Err(cause) = client.recording().toggle().await {
                            tracing::error!(?cause, "failed to toggle recording");
                        }
                    }
                    RecordingAction::Start => {
                        if let Err(cause) = client.recording().start().await {
                            tracing::error!(?cause, "failed to start recording");
                        }
                    }
                    RecordingAction::Stop => {
                        if let Err(cause) = client.recording().stop().await {
                            tracing::error!(?cause, "failed to stop recording");
                        }
                    }
                    RecordingAction::PauseResume => {
                        if let Err(cause) = client.recording().toggle_pause().await {
                            tracing::error!(?cause, "failed to toggle recording pause");
                        }
                    }
                    RecordingAction::Pause => {
                        if let Err(cause) = client.recording().pause().await {
                            tracing::error!(?cause, "failed to pause recording");
                        }
                    }
                    RecordingAction::Resume => {
                        if let Err(cause) = client.recording().resume().await {
                            tracing::error!(?cause, "failed to resume recording");
                        }
                    }
                });
            }
            Action::Streaming(properties) => {
                let action: StreamAction = match properties.action {
                    Some(value) => value,
                    None => return,
                };

                run_with_client(self.state.clone(), async move |client| match action {
                    StreamAction::StartStop => {
                        if let Err(cause) = client.streaming().toggle().await {
                            tracing::error!(?cause, "failed to toggle streaming");
                        }
                    }
                    StreamAction::Start => {
                        if let Err(cause) = client.streaming().start().await {
                            tracing::error!(?cause, "failed to start streaming");
                        }
                    }
                    StreamAction::Stop => {
                        if let Err(cause) = client.streaming().stop().await {
                            tracing::error!(?cause, "failed to stop streaming");
                        }
                    }
                });
            }
            Action::VirtualCamera(properties) => {
                let action: VirtualCameraAction = match properties.action {
                    Some(value) => value,
                    None => return,
                };

                run_with_client(self.state.clone(), async move |client| match action {
                    VirtualCameraAction::StartStop => {
                        if let Err(cause) = client.virtual_cam().toggle().await {
                            tracing::error!(?cause, "failed to toggle virtual camera");
                        }
                    }
                    VirtualCameraAction::Start => {
                        if let Err(cause) = client.virtual_cam().start().await {
                            tracing::error!(?cause, "failed to start virtual camera");
                        }
                    }
                    VirtualCameraAction::Stop => {
                        if let Err(cause) = client.virtual_cam().stop().await {
                            tracing::error!(?cause, "failed to stop virtual camera");
                        }
                    }
                });
            }
            Action::SwitchScene(properties) => {
                let scene = match properties.scene {
                    Some(value) => value,
                    None => return,
                };

                let scene_id = match Uuid::from_str(&scene) {
                    Ok(value) => value,
                    Err(_) => return,
                };

                run_with_client(self.state.clone(), async move |client| {
                    let scenes = client.scenes();

                    if let Err(cause) = scenes
                        .set_current_program_scene(SceneId::Uuid(scene_id))
                        .await
                    {
                        tracing::error!(?cause, "failed to set current scene");
                    }
                });
            }
            Action::SwitchProfile(properties) => {
                let profile = match properties.profile {
                    Some(value) => value,
                    None => return,
                };

                run_with_client(self.state.clone(), async move |client| {
                    let profiles = client.profiles();
                    if let Err(cause) = profiles.set_current(&profile).await {
                        tracing::error!(?cause, "failed to set current profile");
                    }
                });
            }
        }
    }
}

/// Run the provided action in a local background task using the
/// obs client (Only runs if the client is connected)
fn run_with_client<F>(state: Rc<State>, action: F)
where
    F: for<'a> AsyncFnOnce(&'a mut obws::Client) -> (),
    F: 'static,
{
    spawn_local(async move {
        let client = &mut *state.client_state.lock().await;
        let client = match client {
            ClientState::Connected { client } => client,
            _ => return,
        };

        action(client).await;
    });
}

enum Action {
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
struct SwitchSceneProperties {
    scene: Option<String>,
}

#[derive(Deserialize)]
struct SwitchProfileProperties {
    profile: Option<String>,
}

#[derive(Deserialize)]
struct RecordingActionProperties {
    action: Option<RecordingAction>,
}

#[derive(Deserialize)]
enum RecordingAction {
    StartStop,
    Start,
    Stop,
    PauseResume,
    Pause,
    Resume,
}

#[derive(Deserialize)]
struct StreamActionProperties {
    action: Option<StreamAction>,
}

#[derive(Deserialize)]
enum StreamAction {
    StartStop,
    Start,
    Stop,
}

#[derive(Deserialize)]
struct VirtualCameraActionProperties {
    action: Option<VirtualCameraAction>,
}

#[derive(Deserialize)]
enum VirtualCameraAction {
    StartStop,
    Start,
    Stop,
}
