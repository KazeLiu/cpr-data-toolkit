use gateway_plugin_sdk::{
    ErrorCode, PluginFault, call::management::ManagementResponse, client::TypedReply,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;

pub type Result<T> = std::result::Result<T, ApiError>;
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}
impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(400, "invalid_input", message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "conflict", message)
    }
    pub fn reply(self) -> std::result::Result<TypedReply<ManagementResponse>, PluginFault> {
        reply(
            self.status,
            &json!({"error":{"code":self.code,"message":self.message}}),
        )
    }
}
impl From<PluginFault> for ApiError {
    fn from(error: PluginFault) -> Self {
        match error.code {
            ErrorCode::Conflict => Self::conflict("数据已更新，请重新读取后确认"),
            ErrorCode::InvalidInput => Self::invalid("宿主拒绝了操作参数"),
            ErrorCode::PermissionDenied => Self::new(403, "denied", "宿主未授权此操作"),
            _ => Self::new(502, "host_unavailable", "宿主操作未完成，请核对结果"),
        }
    }
}
impl From<serde_json::Error> for ApiError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(500, "encoding", "数据格式处理失败")
    }
}
impl From<gateway_plugin_sdk::call::services::ServiceError> for ApiError {
    fn from(error: gateway_plugin_sdk::call::services::ServiceError) -> Self {
        let status = match error.kind.as_str() {
            "invalid_input" | "capacity_exceeded" => 400,
            "not_found" => 404,
            "conflict" => 409,
            _ => 502,
        };
        Self::new(
            status,
            "usage_service",
            if status == 502 {
                "宿主用量操作结果未确认，请保留本次任务并继续核对".into()
            } else {
                error.message
            },
        )
    }
}
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| ApiError::invalid("请求正文格式不正确"))
}
pub fn reply(
    status: u16,
    body: &impl Serialize,
) -> std::result::Result<TypedReply<ManagementResponse>, PluginFault> {
    let payload =
        serde_json::to_vec(body).map_err(|_| PluginFault::new(ErrorCode::Fault, "响应编码失败"))?;
    Ok(TypedReply::new(ManagementResponse {
        status,
        content_type: "application/json".into(),
        headers: vec![gateway_plugin_sdk::call::middleware::MiddlewareHeader {
            name: "cache-control".into(),
            value: b"no-store".to_vec(),
        }],
    })
    .with_payload(payload))
}
