use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use obws::client::{ConnectConfig, DEFAULT_BROADCAST_CAPACITY};
use serde::{Deserialize, Serialize};
use tilepad_plugin_sdk::{inspector::Inspector, tracing};
use tokio::{
    task::{JoinHandle, spawn_local},
    time::sleep,
};

use crate::messages::InspectorMessageOut;

#[derive(Debug, Default, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClientState {
    #[default]
    Initial,
    NotConnected,
    Connecting,
    RetryConnecting,
    Connected,
    ConnectError,
}

/// Properties for the plugin itself
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Auth {
    pub host: String,
    pub port: u16,
    pub password: String,
}

type ObsError = obws::error::Error;
type ObsClient = obws::Client;

#[derive(Default)]
pub struct State {
    /// Current client state
    client_state: Cell<ClientState>,

    /// Current OBS websocket client instance
    client: tokio::sync::Mutex<Option<ObsClient>>,

    /// Current inspector for sending state updates
    inspector: RefCell<Option<Inspector>>,

    /// Current authentication credentials
    /// (Used when attempting to reconnect)
    current_auth: RefCell<Option<Auth>>,

    /// Handle to a retry task that is attempting to reconnect
    connect_retry_task: RefCell<Option<JoinHandle<()>>>,
}

impl State {
    pub fn set_inspector(&self, inspector: Option<Inspector>) {
        *self.inspector.borrow_mut() = inspector;
    }

    pub fn get_state(&self) -> ClientState {
        self.client_state.get()
    }

    pub fn set_state(&self, state: ClientState) {
        self.client_state.set(state);

        if let Some(inspector) = self.inspector.borrow().as_ref() {
            _ = inspector.send(InspectorMessageOut::ClientState { state });
        }
    }

    // Run some action on the client
    pub fn run_with_client<F>(self: Rc<State>, action: F)
    where
        F: for<'a> AsyncFnOnce(&'a mut obws::Client) -> Result<(), ObsError>,
        F: 'static,
    {
        spawn_local(async move {
            _ = self.execute_with_client(action).await;
        });
    }

    pub fn queue_background_retry(self: Rc<Self>, auth: Auth) {
        if self.connect_retry_task.borrow().is_some() {
            return;
        }

        let handle = spawn_local({
            let state = self.clone();
            async move {
                loop {
                    // Attempt to connect
                    if state.try_connect(auth.clone(), true).await.is_ok() {
                        state.connect_retry_task.replace(None);
                        break;
                    }

                    // Wait for next attempt
                    sleep(Duration::from_secs(10)).await;
                }
            }
        });

        self.connect_retry_task.replace(Some(handle));
    }

    pub async fn try_connect(&self, auth: Auth, retry: bool) -> Result<(), ObsError> {
        if retry {
            self.set_state(ClientState::RetryConnecting);
        } else {
            // Stop any current retry tasks
            if let Some(task) = self.connect_retry_task.borrow_mut().take() {
                task.abort();
            }

            self.set_state(ClientState::Connecting);
        }

        // Remove password if its empty
        let mut password: Option<String> = None;
        if !auth.password.trim().is_empty() {
            password = Some(auth.password.clone())
        }

        let config = ConnectConfig {
            host: &auth.host,
            port: auth.port,
            dangerous: None,
            password,
            event_subscriptions: None,
            broadcast_capacity: DEFAULT_BROADCAST_CAPACITY,
            connect_timeout: Duration::from_secs(5),
        };

        let client = match obws::Client::connect_with_config(config).await {
            Ok(value) => value,
            Err(error) => {
                self.set_state(ClientState::ConnectError);
                tracing::error!(?error, "failed to connect");
                return Err(error);
            }
        };

        let mut client_lock = self.client.lock().await;
        *client_lock = Some(client);

        // Persist the current credentials
        self.current_auth.replace(Some(auth));
        self.set_state(ClientState::Connected);

        Ok(())
    }

    // Execute an action with the client, handles updating the client state
    // in the event of a disconnect or error
    async fn execute_with_client<F, O>(self: Rc<Self>, action: F) -> Result<Option<O>, ObsError>
    where
        F: for<'a> AsyncFnOnce(&'a mut obws::Client) -> Result<O, ObsError>,
        F: 'static,
    {
        let mut client_lock = self.client.lock().await;

        // Acquire the client access
        let client = match client_lock.as_mut() {
            Some(value) => value,
            None => return Ok(None),
        };

        match action(client).await {
            Ok(value) => Ok(Some(value)),
            Err(err) => {
                match &err {
                    // We've lost connection or something of the sort
                    ObsError::Send(_) => {
                        // Clear the client lock value then drop it
                        {
                            *client_lock = None;
                            drop(client_lock);
                        }

                        // Update connection state
                        self.client_state.replace(ClientState::NotConnected);

                        // Queue retry connect attempt
                        let auth = self.current_auth.borrow().clone();
                        if let Some(auth) = auth {
                            self.queue_background_retry(auth);
                        }
                    }

                    cause => {
                        tracing::error!(?cause, "unhandled obs error");
                    }
                }

                Err(err)
            }
        }
    }
}
