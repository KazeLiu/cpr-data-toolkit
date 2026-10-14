mod accounts;
mod api;
mod host;
mod management;
mod model_ua;
mod pricing;
mod sources;
mod state;

use gateway_plugin_sdk::client::{PluginBuilder, PluginSession, SessionConfig, SessionError};
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = PluginSession::accept(
        tokio::io::stdin(),
        tokio::io::stdout(),
        SessionConfig::default(),
    )
    .await?;
    let builder = PluginBuilder::from_json(include_bytes!("../plugin.json"))?;
    if session.handshake().plugin_id != "kazeliu.cpr-data-toolkit" {
        return Err(SessionError::Handshake.into());
    }
    let sources = Arc::new(sources::SourceStore::default());
    let logs = Arc::new(model_ua::RequestLogs::default());
    let middleware_logs = Arc::clone(&logs);
    let cleanup = Arc::clone(&sources);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup.expire().await;
        }
    });
    let plugin = builder
        .middleware(move |call| {
            let logs = Arc::clone(&middleware_logs);
            async move { model_ua::middleware(&logs, call).await }
        })?
        .management(management::registration(), move |call| {
            let sources = Arc::clone(&sources);
            let logs = Arc::clone(&logs);
            async move { management::handle(&sources, &logs, call).await }
        })?
        .build()?;
    session.run(plugin).await?;
    Ok(())
}
