use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    client::{HostClient, SessionError},
};
use serde::{Serialize, de::DeserializeOwned};

pub async fn metadata<I: Serialize, O: DeserializeOwned>(
    host: &HostClient,
    method: &str,
    input: &I,
) -> std::result::Result<O, PluginFault> {
    let reply = host
        .call(
            method,
            serde_json::to_value(input).map_err(|_| invalid())?,
            vec![],
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    if !reply.payload.is_empty() {
        return Err(invalid());
    }
    serde_json::from_value(reply.result).map_err(|_| invalid())
}
pub async fn binary<I: Serialize, O: DeserializeOwned>(
    host: &HostClient,
    method: &str,
    input: &I,
) -> std::result::Result<O, PluginFault> {
    let reply = host
        .call(
            method,
            serde_json::json!({}),
            serde_json::to_vec(input).map_err(|_| invalid())?,
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    if reply.result != serde_json::json!({}) {
        return Err(invalid());
    }
    serde_json::from_slice(&reply.payload).map_err(|_| invalid())
}
fn invalid() -> PluginFault {
    PluginFault::new(ErrorCode::Fault, "宿主回调数据格式无效")
}
