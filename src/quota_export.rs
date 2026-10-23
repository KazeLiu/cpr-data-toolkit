//! 在宿主已鉴权的请求链中提供插件额度透传，范围与匿名化由公开 SDK 执行

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        middleware::{MiddlewareBodyFrame, MiddlewareBodyFraming, MiddlewareHeader},
        services::quota::{ExportQuota, QuotaSharingQuery},
    },
    client::{MiddlewareBody, MiddlewareResponse, RequestCall},
};
use serde_json::json;

pub async fn handle(call: RequestCall) -> Result<MiddlewareResponse, PluginFault> {
    let result = call
        .host
        .service::<ExportQuota>(QuotaSharingQuery {
            client_key_id: call.request.head.client_key_id.clone(),
        })
        .await;
    let (status, body) = match result {
        Ok(quota) => (200, serde_json::to_value(quota)),
        Err(error) => {
            let (status, kind, code) = match error.kind.as_str() {
                "unauthorized" => (401, "authentication_error", "invalid_api_key"),
                "forbidden" => (403, "permission_error", "quota_sharing_disabled"),
                _ => (503, "server_error", "quota_unavailable"),
            };
            (
                status,
                Ok(json!({"error":{"message":error.message,"type":kind,"code":code}})),
            )
        }
    };
    let bytes = body
        .and_then(|body| serde_json::to_vec(&body))
        .map_err(|_| PluginFault::new(ErrorCode::Fault, "额度响应编码失败"))?;
    Ok(MiddlewareResponse::direct(
        "openai",
        status,
        vec![
            MiddlewareHeader {
                name: "content-type".into(),
                value: b"application/json".to_vec(),
            },
            MiddlewareHeader {
                name: "cache-control".into(),
                value: b"no-store".to_vec(),
            },
        ],
        MiddlewareBody::from_frames(
            MiddlewareBodyFraming::JsonDocument,
            vec![MiddlewareBodyFrame::new(bytes, true)],
        ),
    ))
}
