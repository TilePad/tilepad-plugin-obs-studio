use std::{cell::RefCell, rc::Rc};

use serde::{Deserialize, Serialize};
use tilepad_plugin_sdk::{
    inspector::Inspector, plugin::Plugin, protocol::TileInteractionContext,
    session::PluginSessionHandle, tracing,
};
use tokio::{sync::Mutex, task::spawn_local};

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
    Connect { auth: Auth },
}

/// Messages to the inspector
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InspectorMessageOut {
    ClientState { state: String },
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
            Err(cause) => {
                return;
            }
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
        }
    }

    fn on_tile_clicked(
        &self,
        _session: &PluginSessionHandle,
        ctx: TileInteractionContext,
        properties: serde_json::Value,
    ) {
        let action_id = ctx.action_id.as_str();
        match action_id {
            "toggle_recording" => {
                let state = self.state.clone();
                spawn_local(async move {
                    let client = &mut *state.client_state.lock().await;
                    let client = match client {
                        ClientState::Connected { client } => client,
                        _ => return,
                    };
                    client.recording().toggle().await.unwrap();
                });
            }
            action_id => {
                tracing::debug!(?action_id, "unknown tile action requested")
            }
        }
    }
}
