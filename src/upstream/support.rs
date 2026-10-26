//! 只探测额度接口是否存在，不读取身份、不修改状态或访问上游

use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{data, host::ACCOUNT_HTTP},
    client::{HostClient, SessionError},
};
use serde::Serialize;

pub const UPGRADE_HINT: &str =
    "当前宿主尚未提供额度查询或分享接口，请升级到包含 PR #525 接口的 RPC v7 宿主";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Support {
    pub query_available: bool,
    pub sharing_available: bool,
}

pub fn is_missing(error: &PluginFault) -> bool {
    error.code == ErrorCode::Unsupported
        || (error.code == ErrorCode::PermissionDenied
            && error.message == "callback resource is not authorized")
}

pub fn unavailable() -> ApiError {
    ApiError::new(503, "host_quota_unsupported", UPGRADE_HINT)
}

pub fn callback_error(error: PluginFault) -> ApiError {
    if is_missing(&error) {
        unavailable()
    } else {
        error.into()
    }
}

async fn probe(host: &HostClient, method: &str, binary: bool) -> Result<bool> {
    // 缺少必填参数会在接口解析时被拒绝，不进入认证、账号读取或网络发送
    let payload = if binary { b"{}".to_vec() } else { vec![] };
    match host.call(method, serde_json::json!({}), payload).await {
        Ok(_) => Ok(true),
        Err(error) => {
            let error = SessionError::into_plugin_fault(error);
            if error.code == ErrorCode::InvalidInput {
                Ok(true)
            } else if is_missing(&error) {
                Ok(false)
            } else {
                Err(error.into())
            }
        }
    }
}

pub async fn query_available(host: &HostClient) -> Result<bool> {
    probe(host, ACCOUNT_HTTP, false).await
}

async fn sharing_available(host: &HostClient) -> Result<bool> {
    let mut available = true;
    for method in [data::CLIENT_VERIFY, data::ACCESS_CHECK, data::QUOTA_DETAILS] {
        available &= probe(host, method, true).await?;
    }
    Ok(available)
}

pub async fn get(host: &HostClient) -> Result<Support> {
    Ok(Support {
        query_available: query_available(host).await?,
        sharing_available: sharing_available(host).await?,
    })
}

pub async fn require_query(host: &HostClient) -> Result<()> {
    if query_available(host).await? {
        Ok(())
    } else {
        Err(unavailable())
    }
}

pub async fn require_sharing(host: &HostClient) -> Result<()> {
    if sharing_available(host).await? {
        Ok(())
    } else {
        Err(unavailable())
    }
}
