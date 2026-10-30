mod account_models;
mod accounts;
mod api;
mod cline;
mod history_pricing;
mod host;
mod key_models;
mod management;
mod model_priority;
mod model_ua;
mod pricing;
mod quota_export;
mod sources;
mod state;
mod upstream;
mod upstream_pin;
mod usage;

use gateway_plugin_sdk::client::{PluginBuilder, PluginSession, SessionConfig, SessionError};
use std::sync::Arc;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session_config = SessionConfig::default();
    let maximum_stream_chunk_bytes = session_config.maximum_stream_chunk_bytes;
    let session =
        PluginSession::accept(tokio::io::stdin(), tokio::io::stdout(), session_config).await?;
    let builder = PluginBuilder::from_json(include_bytes!("../plugin.json"))?;
    if session.handshake().plugin_id != "kazeliu.cpr-data-toolkit" {
        return Err(SessionError::Handshake.into());
    }
    let sources = Arc::new(sources::SourceStore::default());
    let logs = Arc::new(model_ua::RequestLogs::default());
    let usage = Arc::new(usage::Imports::default());
    let priority_logs = Arc::new(model_priority::logs::Logs::default());
    let scheduling_logs = Arc::clone(&priority_logs);
    let observation_logs = Arc::clone(&priority_logs);
    let middleware_logs = Arc::clone(&logs);
    let pin_logs = Arc::new(upstream_pin::Runtime::default());
    let pin_middleware_logs = Arc::clone(&pin_logs);
    let configuration: cline::InstanceConfiguration =
        serde_json::from_value(session.handshake().configuration.clone())?;
    let cline_adapter = Arc::new(cline::Adapter::new(
        maximum_stream_chunk_bytes,
        configuration.cline_account_ids,
    ));
    let cline_registration = Arc::clone(&cline_adapter);
    let cline_execution = Arc::clone(&cline_adapter);
    let cline_logs = Arc::clone(&pin_logs);
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
        .on(
            gateway_plugin_sdk::client::methods::UPSTREAM_ADAPTER_REGISTER,
            move |call| {
                let adapter = Arc::clone(&cline_registration);
                async move { adapter.register(call).await }
            },
        )?
        .on(
            gateway_plugin_sdk::client::methods::UPSTREAM_ADAPTER_EXECUTE,
            move |call| {
                let adapter = Arc::clone(&cline_execution);
                let logs = Arc::clone(&cline_logs);
                async move { adapter.execute(logs, call).await }
            },
        )?
        .on(
            gateway_plugin_sdk::client::methods::SCHEDULE_ACCOUNT,
            move |call| {
                let logs = Arc::clone(&scheduling_logs);
                async move { model_priority::schedule(&logs, call).await }
            },
        )?
        .on(gateway_plugin_sdk::client::methods::OBSERVE, move |call| {
            let logs = Arc::clone(&observation_logs);
            async move {
                if let gateway_plugin_sdk::call::observation::Event::RequestCompleted(event) =
                    call.request
                {
                    logs.completed(&event).await;
                }
                Ok(gateway_plugin_sdk::client::TypedReply::new(
                    gateway_plugin_sdk::client::Empty {},
                ))
            }
        })?
        .on(
            gateway_plugin_sdk::client::methods::RECONCILE,
            |call| async move {
                upstream::maintain(&call.host).await.map_err(|_| {
                    gateway_plugin_sdk::PluginFault::new(
                        gateway_plugin_sdk::ErrorCode::Fault,
                        "额度后台刷新未完成",
                    )
                })?;
                Ok(gateway_plugin_sdk::client::TypedReply::new(
                    gateway_plugin_sdk::client::Empty {},
                ))
            },
        )?
        .middleware(move |call: gateway_plugin_sdk::client::MiddlewareCall| {
            let logs = Arc::clone(&middleware_logs);
            let pin_logs = Arc::clone(&pin_middleware_logs);
            async move {
                match call {
                    gateway_plugin_sdk::client::MiddlewareCall::Http(call)
                        if call.request.uri.split('?').next() == Some("/v1/quota") =>
                    {
                        quota_export::handle(*call).await.map(Into::into)
                    }
                    gateway_plugin_sdk::client::MiddlewareCall::Outbound(call) => {
                        upstream_pin::outbound(&pin_logs, *call)
                            .await
                            .map(Into::into)
                    }
                    gateway_plugin_sdk::client::MiddlewareCall::Request(call)
                        if call.context.stage == gateway_plugin_sdk::Stage::Attempt =>
                    {
                        upstream_pin::observe(pin_logs, *call).await.map(Into::into)
                    }
                    gateway_plugin_sdk::client::MiddlewareCall::Request(call) => {
                        key_models::middleware(&logs, *call).await.map(Into::into)
                    }
                    call => call.forward().await,
                }
            }
        })?
        .management(management::registration(), move |call| {
            let sources = Arc::clone(&sources);
            let logs = Arc::clone(&logs);
            let usage = Arc::clone(&usage);
            let priority_logs = Arc::clone(&priority_logs);
            let pin_logs = Arc::clone(&pin_logs);
            let cline_adapter = Arc::clone(&cline_adapter);
            async move {
                management::handle(
                    &sources,
                    &logs,
                    &usage,
                    &priority_logs,
                    &pin_logs,
                    &cline_adapter,
                    call,
                )
                .await
            }
        })?
        .build()?;
    session.run(plugin).await?;
    Ok(())
}
