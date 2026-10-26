//! 插件查询协议与受管账号 HTTP；一次刷新共用期限，失败时释放响应流

use super::{
    parse,
    types::{QuotaQueryMode, QuotaResponse},
};
use gateway_plugin_sdk::{
    ErrorCode,
    call::host::{AccountHttpRequest, AuthCredential, HttpRequest},
    client::HostClient,
};
use serde_json::{Value, json};
use std::time::Duration;

pub(super) fn parse_base_url(value: &str) -> Result<url::Url, &'static str> {
    let url = url::Url::parse(value).map_err(|_| "invalid_response")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid_response");
    }
    Ok(url)
}

fn endpoint(base: &str, suffix: &str) -> Result<url::Url, &'static str> {
    let mut url = parse_base_url(base)?;
    let path = url.path().trim_end_matches('/');
    let root = path.strip_suffix("/v1").unwrap_or(path);
    url.set_path(&format!("{root}/v1/{suffix}"));
    Ok(url)
}

struct Query<'a> {
    host: &'a HostClient,
    credential: &'a AuthCredential,
    key: &'a str,
    deadline: tokio::time::Instant,
}

pub(super) async fn fetch(
    host: &HostClient,
    credential: &AuthCredential,
    mode: QuotaQueryMode,
) -> Result<QuotaResponse, &'static str> {
    let material = &credential.facts.material;
    let base = material
        .get("base_url")
        .and_then(Value::as_str)
        .ok_or("invalid_response")?;
    let key = material
        .get("api_key")
        .and_then(Value::as_str)
        .ok_or("invalid_response")?;
    let seconds = match mode {
        QuotaQueryMode::Cpr => 5,
        QuotaQueryMode::KimiCoding
        | QuotaQueryMode::KimiCodingGlobal
        | QuotaQueryMode::CommandCode => 15,
        _ => 10,
    };
    let query = Query {
        host,
        credential,
        key,
        deadline: tokio::time::Instant::now() + Duration::from_secs(seconds),
    };
    let mut response = if mode == QuotaQueryMode::CommandCode {
        let whoami = query
            .command_json("/alpha/whoami", &[("limits", "1")])
            .await?;
        let org = whoami.pointer("/org/id").and_then(Value::as_str);
        let params = org.map(|id| vec![("orgId", id)]).unwrap_or_default();
        let credits = query
            .command_json("/alpha/billing/credits", &params)
            .await?;
        let subscription = query
            .command_json("/alpha/billing/subscriptions", &params)
            .await?;
        let mut params = params;
        if let Some(start) = subscription
            .pointer("/data/currentPeriodStart")
            .and_then(Value::as_str)
        {
            params.push(("since", start));
        }
        let summary = query.command_json("/alpha/usage/summary", &params).await?;
        parse::parse(
            &json!({"credits":credits,"subscription":subscription,"summary":summary}),
            mode,
            super::now(),
        )?
    } else {
        let url = if mode == QuotaQueryMode::Cpr {
            endpoint(base, "quota")?
        } else {
            parse::query_url(base, mode)?
        };
        let value = query
            .get(
                url,
                if mode == QuotaQueryMode::Cpr {
                    1024 * 1024
                } else {
                    256 * 1024
                },
            )
            .await?;
        if mode == QuotaQueryMode::Cpr {
            serde_json::from_value(value).map_err(|_| "invalid_response")?
        } else {
            parse::parse(&value, mode, super::now())?
        }
    };
    response.validate().map_err(|_| "invalid_response")?;
    if mode != QuotaQueryMode::Cpr {
        let models = async {
            let value = query.get(endpoint(base, "models")?, 256 * 1024).await?;
            let list = value["data"].as_array().ok_or("invalid_response")?;
            list.iter()
                .map(|model| {
                    model["id"]
                        .as_str()
                        .map(str::to_owned)
                        .ok_or("invalid_response")
                })
                .collect::<Result<Vec<_>, _>>()
        }
        .await;
        let models = match models {
            Ok(models) => Some(models),
            Err(code @ ("unauthorized" | "forbidden")) => return Err(code),
            _ => None,
        };
        for account in &mut response.accounts {
            account.label = credential.facts.name.clone();
            account.models = models.clone();
        }
    }
    response.validate().map_err(|_| "invalid_response")?;
    Ok(response)
}

impl Query<'_> {
    async fn command_json(
        &self,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, &'static str> {
        let mut url = parse_base_url("https://api.commandcode.ai")?;
        url.set_path(path);
        url.query_pairs_mut().extend_pairs(params.iter().copied());
        self.get(url, 256 * 1024).await
    }

    async fn get(&self, url: url::Url, maximum: usize) -> Result<Value, &'static str> {
        if tokio::time::Instant::now() >= self.deadline {
            return Err("timeout");
        }
        let request = self.host.account_http(AccountHttpRequest {
            account_id: self.credential.account_id.clone(),
            credential_revision: self.credential.credential_revision,
            request: HttpRequest {
                method: "GET".into(),
                url: url.into(),
                headers: vec![
                    (
                        "authorization".into(),
                        format!("Bearer {}", self.key).into_bytes(),
                    ),
                    ("accept".into(), b"application/json".to_vec()),
                ],
            },
            body: vec![],
        });
        let mut response = tokio::time::timeout_at(self.deadline, request)
            .await
            .map_err(|_| "timeout")?
            .map_err(|error| match error.code {
                ErrorCode::Timeout => "timeout",
                ErrorCode::Conflict => "conflict",
                _ => "network_error",
            })?;
        let result = async {
            match response.status {
                200..=299 => (),
                401 => return Err("unauthorized"),
                403 => return Err("forbidden"),
                404 | 405 => return Err("unsupported"),
                429 => return Err("rate_limited"),
                _ => return Err("upstream_error"),
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = tokio::time::timeout_at(self.deadline, response.body.read())
                .await
                .map_err(|_| "timeout")?
                .map_err(|_| "network_error")?
            {
                if chunk.len() > maximum.saturating_sub(bytes.len()) {
                    return Err("response_too_large");
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&bytes).map_err(|_| "invalid_response")
        }
        .await;
        if result.is_err() {
            // 清理预算独立于查询期限；不能用关闭失败覆盖已经收到的 401/403
            let _ = tokio::time::timeout(Duration::from_secs(1), response.body.close()).await;
        }
        result
    }
}
