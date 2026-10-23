mod accounts;
mod api;
mod history_pricing;
mod host;
mod management;
mod model_ua;
mod pricing;
mod quota_export;
mod sources;
mod state;
mod upstream;
mod usage;

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
    let usage = Arc::new(usage::Imports::default());
    let middleware_logs = Arc::clone(&logs);
    let cleanup = Arc::clone(&sources);
    let import_cleanup = Arc::clone(&usage);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            cleanup.expire().await;
            import_cleanup.expire().await;
        }
    });
    let plugin = builder
        .middleware(move |call: gateway_plugin_sdk::client::RequestCall| {
            let logs = Arc::clone(&middleware_logs);
            async move {
                if call.request.head.endpoint == "/v1/quota" {
                    quota_export::handle(call).await
                } else {
                    model_ua::middleware(&logs, call).await
                }
            }
        })?
        .management(management::registration(), move |call| {
            let sources = Arc::clone(&sources);
            let logs = Arc::clone(&logs);
            let usage = Arc::clone(&usage);
            async move { management::handle(&sources, &logs, &usage, call).await }
        })?
        .build()?;
    session.run(plugin).await?;
    Ok(())
}
