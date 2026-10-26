//! 插件 HTTP 额度分享；认证与授权事实来自宿主，披露和匿名化由插件处理

use crate::{
    api::{ApiError, Result},
    upstream::{
        self, support,
        types::{
            MAX_QUOTA_RESPONSE_BYTES, QuotaAccount, QuotaBalance, QuotaObservationStatus,
            QuotaResponse, QuotaWindow, QuotaWindowRole,
        },
    },
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        data::{AccessQuery, QuotaFactsQuery, VerifyClientRequest},
        middleware::MiddlewareHeader,
    },
    client::{HostClient, HttpBody, HttpCall, HttpFrame, HttpResponse},
};
use serde_json::json;
use std::collections::BTreeMap;

pub async fn handle(mut call: HttpCall) -> std::result::Result<HttpResponse, PluginFault> {
    let result = async {
        let uri: http::Uri = call
            .request
            .uri
            .parse()
            .map_err(|_| ApiError::invalid("查询地址无效"))?;
        if call.request.method != "GET" {
            return Err(ApiError::new(405, "method_not_allowed", "只支持 GET"));
        }
        if uri.query().is_some() {
            return Err(ApiError::invalid("额度查询不接受查询参数"));
        }
        let authorization = call
            .request
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case("authorization"))
            .and_then(|header| std::str::from_utf8(&header.value).ok())
            .ok_or_else(unauthorized)?;
        let identity = call
            .host
            .verify_client(VerifyClientRequest {
                authorization: authorization.into(),
            })
            .await
            .map_err(|error| {
                if support::is_missing(&error) {
                    support::unavailable()
                } else if matches!(
                    error.code,
                    ErrorCode::PermissionDenied | ErrorCode::Rejected | ErrorCode::InvalidInput
                ) {
                    unauthorized()
                } else {
                    ApiError::new(503, "quota_unavailable", "宿主认证暂不可用")
                }
            })?;
        while let Some(frame) = call.request.body.read().await? {
            if matches!(frame, HttpFrame::Data(ref data) if !data.is_empty()) {
                return Err(ApiError::invalid("额度查询不接受请求正文"));
            }
        }
        export(&call.host, &identity.client_key_id)
            .await
            .map_err(|error| {
                if error.status >= 500 && error.code != "host_quota_unsupported" {
                    ApiError::new(503, "quota_unavailable", error.message)
                } else {
                    error
                }
            })
    }
    .await;
    // 无论认证或格式是否失败，都显式释放请求正文
    call.request.body.close().await?;
    let (status, body) = match result {
        Ok(response) => (200, serde_json::to_value(response)),
        Err(error) => (
            error.status,
            Ok(
                json!({"error":{"message":error.message,"code":error.code,"type":if error.status == 401 { "authentication_error" } else if error.status == 403 { "permission_error" } else { "server_error" }}}),
            ),
        ),
    };
    let bytes = body
        .and_then(|body| serde_json::to_vec(&body))
        .map_err(|_| PluginFault::new(ErrorCode::Fault, "额度响应编码失败"))?;
    let mut response = HttpResponse::new(status, HttpBody::from_bytes(bytes));
    response.headers = vec![
        MiddlewareHeader {
            name: "content-type".into(),
            value: b"application/json".to_vec(),
        },
        MiddlewareHeader {
            name: "cache-control".into(),
            value: b"no-store".to_vec(),
        },
    ];
    Ok(response)
}

fn unauthorized() -> ApiError {
    ApiError::new(401, "invalid_api_key", "API Key 不可用")
}

async fn export(host: &HostClient, key: &str) -> Result<QuotaResponse> {
    let (sharing_version, sharing) = upstream::sharing_record(host, key).await?;
    if !sharing.enabled {
        return Err(ApiError::new(
            403,
            "quota_sharing_disabled",
            "未开启额度分享",
        ));
    }
    let namespace = uuid::Uuid::parse_str(&sharing.salt)
        .map_err(|_| ApiError::new(503, "quota_unavailable", "分享标识不可用"))?;
    let mut accounts = Vec::new();
    for listed in upstream::directory(host).await? {
        let source = upstream::runtime(host, &listed.account_id).await?;
        if !source.enabled
            || matches!(
                source.credential_state.as_str(),
                "invalid" | "banned" | "expired"
            )
        {
            continue;
        }
        let allowed = host
            .check_access(AccessQuery {
                client_key_id: key.into(),
                account_ids: vec![source.account_id.clone()],
                upstream_model: None,
            })
            .await
            .map_err(support::callback_error)?;
        if !allowed.accounts[0].allowed {
            continue;
        }
        let entries = if source.authentication_kind == "api_key" {
            upstream::export_accounts(host, &source).await?
        } else {
            let quota = host
                .quota_details(QuotaFactsQuery {
                    account_id: source.account_id.clone(),
                })
                .await
                .map_err(support::callback_error)?;
            let observed_at = quota
                .observed_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis);
            let windows: Vec<_> = quota
                .windows
                .into_iter()
                .map(|window| QuotaWindow {
                    key: window.key,
                    group: window.group,
                    label: window.label,
                    role: match window.role.as_deref() {
                        Some("primary") => Some(QuotaWindowRole::Primary),
                        Some("secondary") => Some(QuotaWindowRole::Secondary),
                        Some("monthly") => Some(QuotaWindowRole::Monthly),
                        _ => None,
                    },
                    window_seconds: window.window_seconds,
                    used_percent: window.used_percent,
                    reset_at: window
                        .reset_at_ms
                        .and_then(chrono::DateTime::from_timestamp_millis),
                    limit_reached: window.limit_reached,
                    limit_id: window.limit_id,
                    limit_name: window.limit_name,
                })
                .collect();
            let balances = quota
                .credits
                .map(|credits| QuotaBalance {
                    label: "点数余额".into(),
                    remaining: credits.balance,
                    used: None,
                    total: None,
                    unit: None,
                    unlimited: credits.unlimited,
                    is_valid: credits.has_credits || credits.unlimited,
                    invalid_message: None,
                    window_seconds: None,
                    reset_at: None,
                    details: vec![],
                })
                .into_iter()
                .collect::<Vec<_>>();
            vec![QuotaAccount {
                id: "native".into(),
                label: source.name.clone(),
                models: None,
                status: if let Some(at) = observed_at {
                    if (upstream::now() - at).num_seconds() >= 600 {
                        QuotaObservationStatus::Stale
                    } else {
                        QuotaObservationStatus::Observed
                    }
                } else {
                    QuotaObservationStatus::Unknown
                },
                observed_at,
                windows: if observed_at.is_some() {
                    windows
                } else {
                    vec![]
                },
                balances: if observed_at.is_some() {
                    balances
                } else {
                    vec![]
                },
            }]
        };
        // 同一来源的多条快照可能共享模型，复用本次冻结身份下已经检查过的结果
        let mut model_access = BTreeMap::new();
        for mut entry in entries {
            if sharing.include_models {
                if let Some(models) = entry.models.take() {
                    let mut visible = Vec::new();
                    for model in models {
                        let allowed = if let Some(allowed) = model_access.get(&model) {
                            *allowed
                        } else {
                            let checked = host
                                .check_access(AccessQuery {
                                    client_key_id: key.into(),
                                    account_ids: vec![source.account_id.clone()],
                                    upstream_model: Some(model.clone()),
                                })
                                .await
                                .map_err(support::callback_error)?;
                            let allowed = checked.accounts[0].allowed;
                            model_access.insert(model.clone(), allowed);
                            allowed
                        };
                        if allowed {
                            visible.push(model);
                        }
                    }
                    entry.models = Some(visible);
                }
            } else {
                entry.models = None;
            }
            let input = serde_json::to_vec(&(key, &source.account_id, &entry.id))?;
            entry.id = uuid::Uuid::new_v5(&namespace, &input).to_string();
            if !sharing.include_names {
                entry.label = format!("上游账号 {}", accounts.len() + 1);
            }
            accounts.push(entry);
        }
    }
    if upstream::sharing_record(host, key).await?.0 != sharing_version {
        return Err(ApiError::conflict("分享设置已变更，请重新查询"));
    }
    let response = QuotaResponse {
        schema_version: 1,
        accounts,
    };
    response
        .validate()
        .map_err(|_| ApiError::new(503, "quota_unavailable", "额度快照格式无效"))?;
    if serde_json::to_vec(&response)?.len() > MAX_QUOTA_RESPONSE_BYTES {
        return Err(ApiError::new(
            503,
            "quota_unavailable",
            "额度响应超过大小限制",
        ));
    }
    Ok(response)
}
