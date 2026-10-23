//! 通过公开 SDK 管理宿主额度查询、读取快照与配置分享

use crate::{
    api::{ApiError, Result},
    host,
};
use gateway_plugin_sdk::{
    call::{
        host::{AuthListRequest, AuthListResult},
        services::{
            ServiceError,
            quota::{
                AccountQuotaQuery, AccountQuotaSnapshot, GetAccountQuota, GetQuotaQuerySettings,
                GetQuotaSharing, QuotaQueryMode, QuotaQuerySettings, QuotaSharingQuery,
                QuotaSharingSettings, RefreshAccountQuota, SetQuotaQuerySettings, SetQuotaSharing,
                UpdateQuotaQuerySettings, UpdateQuotaSharing,
            },
        },
    },
    client::HostClient,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    account_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    account_id: String,
    expected_version: u64,
    query_type: QuotaQueryMode,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SharingSelection {
    client_key_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveSharing {
    client_key_id: String,
    expected_config_revision: u64,
    enabled: bool,
    include_names: bool,
    include_models: bool,
}

pub async fn accounts(host: &HostClient) -> Result<Value> {
    let mut cursor = None;
    let mut accounts = Vec::new();
    loop {
        let page: AuthListResult = host::binary(
            host,
            "host.auth.list",
            &AuthListRequest {
                provider_id: Some("openai".into()),
                cursor,
                limit: 200,
            },
        )
        .await?;
        accounts.extend(
            page.accounts
                .into_iter()
                .filter(|account| account.authentication_kind == "api_key")
                .map(|account| {
                    json!({
                        "id": account.account_id,
                        "name": account.name,
                        "enabled": account.enabled,
                    })
                }),
        );
        if accounts.len() > 5000 {
            return Err(ApiError::invalid("账号目录超过 5000 项，请缩小账号范围"));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(json!({"accounts":accounts}))
}

pub fn service_error(error: ServiceError) -> ApiError {
    let status = match error.kind.as_str() {
        "invalid" | "invalid_input" => 400,
        "unauthorized" => 401,
        "forbidden" => 403,
        "not_found" => 404,
        "conflict" => 409,
        "rate_limited" => 429,
        _ => 502,
    };
    ApiError::new(status, "quota_service", error.message)
}

fn query_error(code: &str) -> &str {
    match code {
        "unsupported" => "上游不支持额度查询",
        "sharing_disabled" => "上游未开放额度共享",
        "unauthorized" => "上游额度查询认证失败",
        "forbidden" => "没有上游额度查询权限",
        "timeout" => "上游额度查询超时",
        "invalid_response" => "上游额度格式不兼容",
        "response_too_large" => "上游额度响应过大",
        "rate_limited" => "上游额度查询频率受限",
        _ => "上游额度获取失败",
    }
}

fn view(
    settings: QuotaQuerySettings,
    snapshot: AccountQuotaSnapshot,
    refresh_error: Option<String>,
) -> Result<Value> {
    if settings.credential_revision != snapshot.credential_revision {
        return Err(ApiError::conflict("账号凭据或查询设置已更新，请重新读取"));
    }
    let upstream = snapshot.upstream;
    let error = refresh_error.or_else(|| {
        upstream
            .as_ref()?
            .last_error_code
            .as_deref()
            .map(query_error)
            .map(str::to_owned)
    });
    let accounts = upstream
        .as_ref()
        .map(|quota| {
            quota
                .accounts
                .iter()
                .map(|account| {
                    json!({
                        "id": account.id,
                        "label": account.label,
                        "models": account.models,
                        "status": account.status,
                        "observedAt": account.observed_at,
                        "windows": account.windows,
                        "amounts": account.balances,
                        "message": null,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(json!({
        "version":settings.credential_revision,"queryType":settings.mode,
        "snapshot":upstream.as_ref().and_then(|quota| quota.fetched_at.as_ref()).map(|time| json!({"fetchedAt":time,"accounts":accounts})),
        "lastAttemptAt":upstream.as_ref().and_then(|quota| quota.last_attempt_at.as_ref()),
        "stale":upstream.as_ref().is_some_and(|quota| quota.stale),
        "forwardingBlocked":upstream.as_ref().is_some_and(|quota| quota.forwarding_blocked),
        "error":error,
    }))
}

pub async fn get(host: &HostClient, input: Selection) -> Result<Value> {
    let query = AccountQuotaQuery {
        account_id: input.account_id,
    };
    let settings = host
        .service::<GetQuotaQuerySettings>(query.clone())
        .await
        .map_err(service_error)?;
    let snapshot = host
        .service::<GetAccountQuota>(query)
        .await
        .map_err(service_error)?;
    view(settings, snapshot, None)
}

pub async fn save(host: &HostClient, input: Save) -> Result<Value> {
    let settings = host
        .service::<SetQuotaQuerySettings>(UpdateQuotaQuerySettings {
            account_id: input.account_id.clone(),
            expected_credential_revision: input.expected_version,
            mode: input.query_type,
        })
        .await
        .map_err(service_error)?;
    let snapshot = host
        .service::<GetAccountQuota>(AccountQuotaQuery {
            account_id: input.account_id,
        })
        .await
        .map_err(service_error)?;
    view(settings, snapshot, None)
}

pub async fn refresh(host: &HostClient, input: Selection) -> Result<Value> {
    let query = AccountQuotaQuery {
        account_id: input.account_id,
    };
    let settings = host
        .service::<GetQuotaQuerySettings>(query.clone())
        .await
        .map_err(service_error)?;
    let result = host
        .service::<RefreshAccountQuota>(query)
        .await
        .map_err(service_error)?;
    view(
        settings,
        result.snapshot,
        result.error.map(|error| error.message),
    )
}
pub async fn sharing(host: &HostClient, input: SharingSelection) -> Result<Value> {
    let result = host
        .service::<GetQuotaSharing>(QuotaSharingQuery {
            client_key_id: input.client_key_id,
        })
        .await
        .map_err(service_error)?;
    Ok(
        json!({"configRevision":result.config_revision,"enabled":result.settings.enabled,
        "includeNames":result.settings.include_names,"includeModels":result.settings.include_models}),
    )
}

pub async fn save_sharing(host: &HostClient, input: SaveSharing) -> Result<Value> {
    let result = host
        .service::<SetQuotaSharing>(UpdateQuotaSharing {
            client_key_id: input.client_key_id,
            expected_config_revision: input.expected_config_revision,
            settings: QuotaSharingSettings {
                enabled: input.enabled,
                include_names: input.include_names,
                include_models: input.include_models,
            },
        })
        .await
        .map_err(service_error)?;
    Ok(
        json!({"configRevision":result.config_revision,"enabled":result.settings.enabled,
        "includeNames":result.settings.include_names,"includeModels":result.settings.include_models}),
    )
}
