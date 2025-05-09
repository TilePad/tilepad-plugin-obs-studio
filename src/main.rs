use plugin::ObsPlugin;
use tilepad_plugin_sdk::{setup_tracing, start_plugin};
use tokio::task::LocalSet;

mod action;
mod messages;
mod plugin;
mod state;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    setup_tracing();

    let local_set = LocalSet::new();
    let plugin = ObsPlugin::new();

    local_set.run_until(start_plugin(plugin)).await;
}
