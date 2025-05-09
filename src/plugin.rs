use obws::requests::scenes::SceneId;
use serde::{Deserialize, Serialize};
use std::{rc::Rc, str::FromStr};
use tilepad_plugin_sdk::{
    inspector::Inspector,
    plugin::Plugin,
    protocol::TileInteractionContext,
    session::PluginSessionHandle,
    tracing::{self},
};
use tokio::task::spawn_local;
use uuid::Uuid;

use crate::{
    action::{Action, RecordingAction, StreamAction, VirtualCameraAction},
    messages::{InspectorMessageIn, InspectorMessageOut, SelectOption},
    state::{Auth, ClientState, State},
};

/// Properties for the plugin itself
#[derive(Debug, Deserialize, Serialize)]
pub struct Properties {
    pub auth: Option<Auth>,
}

#[derive(Default)]
pub struct ObsPlugin {
    state: Rc<State>,
}

impl ObsPlugin {
    pub fn new() -> Self {
        Default::default()
    }
}

impl Plugin for ObsPlugin {
    fn on_properties(&self, _session: &PluginSessionHandle, properties: serde_json::Value) {
        // Nothing to do if already connected
        if matches!(
            self.state.get_state(),
            ClientState::Connecting | ClientState::Connected { .. }
        ) {
            return;
        }

        let properties = match serde_json::from_value::<Properties>(properties) {
            Ok(value) => value,

            // Invalid properties
            Err(_) => {
                self.state.set_state(ClientState::NotConnected);
                return;
            }
        };

        let auth = match properties.auth {
            Some(value) => value,

            // No authentication
            None => {
                self.state.set_state(ClientState::NotConnected);
                return;
            }
        };

        let state = self.state.clone();
        spawn_local(async move {
            if state.try_connect(auth.clone(), false).await.is_err() {
                // Retry connection in the background
                state.queue_background_retry(auth);
            }
        });
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

        match message {
            InspectorMessageIn::GetClientState => {
                _ = inspector.send(InspectorMessageOut::ClientState {
                    state: self.state.get_state(),
                });
            }
            InspectorMessageIn::Connect { auth } => {
                let session = session.clone();
                let state = self.state.clone();

                // Nothing to do if already connected
                if matches!(
                    state.get_state(),
                    ClientState::Connecting | ClientState::Connected { .. }
                ) {
                    return;
                }

                spawn_local(async move {
                    if state.try_connect(auth.clone(), false).await.is_ok() {
                        _ = session.set_properties(Properties { auth: Some(auth) });
                    }
                });
            }
            InspectorMessageIn::GetProfiles => {
                self.state.clone().run_with_client(async move |client| {
                    let profiles = client.profiles();
                    let list = match profiles.list().await {
                        Ok(value) => value,
                        Err(cause) => {
                            tracing::error!(?cause, "failed to get profiles");
                            return Err(cause);
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

                    Ok(())
                });
            }
            InspectorMessageIn::GetScenes => {
                self.state.clone().run_with_client(async move |client| {
                    let scenes = client.scenes();

                    let list = match scenes.list().await {
                        Ok(value) => value,
                        Err(cause) => {
                            tracing::error!(?cause, "failed to get profiles");
                            return Err(cause);
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

                    Ok(())
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

                self.state.clone().run_with_client(async move |client| {
                    match action {
                        RecordingAction::StartStop => {
                            if let Err(cause) = client.recording().toggle().await {
                                tracing::error!(?cause, "failed to toggle recording");
                                return Err(cause);
                            }
                        }
                        RecordingAction::Start => {
                            if let Err(cause) = client.recording().start().await {
                                tracing::error!(?cause, "failed to start recording");
                                return Err(cause);
                            }
                        }
                        RecordingAction::Stop => {
                            if let Err(cause) = client.recording().stop().await {
                                tracing::error!(?cause, "failed to stop recording");
                                return Err(cause);
                            }
                        }
                        RecordingAction::PauseResume => {
                            if let Err(cause) = client.recording().toggle_pause().await {
                                tracing::error!(?cause, "failed to toggle recording pause");
                                return Err(cause);
                            }
                        }
                        RecordingAction::Pause => {
                            if let Err(cause) = client.recording().pause().await {
                                tracing::error!(?cause, "failed to pause recording");
                                return Err(cause);
                            }
                        }
                        RecordingAction::Resume => {
                            if let Err(cause) = client.recording().resume().await {
                                tracing::error!(?cause, "failed to resume recording");
                                return Err(cause);
                            }
                        }
                    }

                    Ok(())
                });
            }
            Action::Streaming(properties) => {
                let action: StreamAction = match properties.action {
                    Some(value) => value,
                    None => return,
                };

                self.state.clone().run_with_client(async move |client| {
                    match action {
                        StreamAction::StartStop => {
                            if let Err(cause) = client.streaming().toggle().await {
                                tracing::error!(?cause, "failed to toggle streaming");
                                return Err(cause);
                            }
                        }
                        StreamAction::Start => {
                            if let Err(cause) = client.streaming().start().await {
                                tracing::error!(?cause, "failed to start streaming");
                                return Err(cause);
                            }
                        }
                        StreamAction::Stop => {
                            if let Err(cause) = client.streaming().stop().await {
                                tracing::error!(?cause, "failed to stop streaming");
                                return Err(cause);
                            }
                        }
                    }

                    Ok(())
                });
            }
            Action::VirtualCamera(properties) => {
                let action: VirtualCameraAction = match properties.action {
                    Some(value) => value,
                    None => return,
                };

                self.state.clone().run_with_client(async move |client| {
                    match action {
                        VirtualCameraAction::StartStop => {
                            if let Err(cause) = client.virtual_cam().toggle().await {
                                tracing::error!(?cause, "failed to toggle virtual camera");
                                return Err(cause);
                            }
                        }
                        VirtualCameraAction::Start => {
                            if let Err(cause) = client.virtual_cam().start().await {
                                tracing::error!(?cause, "failed to start virtual camera");
                                return Err(cause);
                            }
                        }
                        VirtualCameraAction::Stop => {
                            if let Err(cause) = client.virtual_cam().stop().await {
                                tracing::error!(?cause, "failed to stop virtual camera");
                                return Err(cause);
                            }
                        }
                    }

                    Ok(())
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

                self.state.clone().run_with_client(async move |client| {
                    let scenes = client.scenes();

                    if let Err(cause) = scenes
                        .set_current_program_scene(SceneId::Uuid(scene_id))
                        .await
                    {
                        tracing::error!(?cause, "failed to set current scene");
                        return Err(cause);
                    }

                    Ok(())
                });
            }
            Action::SwitchProfile(properties) => {
                let profile = match properties.profile {
                    Some(value) => value,
                    None => return,
                };

                self.state.clone().run_with_client(async move |client| {
                    let profiles = client.profiles();
                    if let Err(cause) = profiles.set_current(&profile).await {
                        tracing::error!(?cause, "failed to set current profile");
                        return Err(cause);
                    }

                    Ok(())
                });
            }
        }
    }
}
