//! 插件自有的上游额度配置、快照、后台刷新与分享设置

mod network;
mod parse;
pub(crate) mod types;

use crate::{
    api::{ApiError, Result},
    host, state,
};
use gateway_plugin_sdk::{
    call::host::{
        AuthCredential, AuthGetRequest, AuthListRequest, AuthListResult, AuthRuntimeAccount,
    },
    client::HostClient,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, Weak},
};
use tokio::sync::Mutex;
use types::{QuotaAccount, QuotaObservationStatus, QuotaQueryMode};

type RefreshLocks = Mutex<BTreeMap<String, Weak<Mutex<()>>>>;
static LOCKS: OnceLock<RefreshLocks> = OnceLock::new();

pub(super) fn now() -> chrono::DateTime<chrono::Utc> {
    std::time::SystemTime::now().into()
}

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

#[derive(Default, Serialize, Deserialize)]
struct Record {
    credential_revision: u64,
    mode: QuotaQueryMode,
    fetched_at: Option<chrono::DateTime<chrono::Utc>>,
    last_attempt_at: Option<chrono::DateTime<chrono::Utc>>,
    error: Option<String>,
    blocked: bool,
    accounts: Vec<QuotaAccount>,
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Sharing {
    pub enabled: bool,
    pub include_names: bool,
    pub include_models: bool,
    pub salt: String,
}

pub(crate) async fn runtime(host: &HostClient, id: &str) -> Result<AuthRuntimeAccount> {
    Ok(host::binary(
        host,
        "host.auth.get_runtime",
        &AuthGetRequest {
            account_id: id.into(),
        },
    )
    .await?)
}

pub(crate) async fn directory(host: &HostClient) -> Result<Vec<AuthRuntimeAccount>> {
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
        accounts.extend(page.accounts);
        if accounts.len() > 5000 {
            return Err(ApiError::invalid("账号目录超过 5000 项"));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(accounts),
        }
    }
}

pub async fn accounts(host: &HostClient) -> Result<Value> {
    Ok(
        json!({"accounts": directory(host).await?.into_iter().filter(|account| account.authentication_kind == "api_key").map(|account| json!({"id":account.account_id,"name":account.name,"enabled":account.enabled})).collect::<Vec<_>>()}),
    )
}

async fn load(host: &HostClient, id: &str) -> Result<(Option<u64>, Record)> {
    let (version, record) = state::get(host, "upstream_quota", id).await?;
    Ok((version, record.unwrap_or_default()))
}

fn view(version: Option<u64>, record: &Record, current_revision: u64) -> Value {
    let valid =
        record.credential_revision == current_revision && record.mode != QuotaQueryMode::Disabled;
    let stale = !valid
        || record.error.is_some()
        || record
            .fetched_at
            .is_none_or(|at| (now() - at).num_seconds() >= 600);
    let mut accounts = if valid {
        record.accounts.clone()
    } else {
        vec![]
    };
    if stale {
        for account in &mut accounts {
            if account.status == QuotaObservationStatus::Observed {
                account.status = QuotaObservationStatus::Stale;
            }
        }
    }
    json!({"version":version.unwrap_or(0),"queryType":record.mode,
        "snapshot": if valid { record.fetched_at.map(|at| json!({"fetchedAt":at,"accounts":accounts.iter().map(|account| json!({"id":account.id,"label":account.label,"models":account.models,"status":account.status,"observedAt":account.observed_at,"windows":account.windows,"amounts":account.balances,"message":null})).collect::<Vec<_>>()})) } else {None},
        "lastAttemptAt":record.last_attempt_at,"stale":stale,"forwardingBlocked":record.mode != QuotaQueryMode::Disabled && (!valid || record.blocked),
        "error": if !valid && record.mode != QuotaQueryMode::Disabled { Some("账号连接已变更，请重新获取额度") } else { record.error.as_deref().map(query_error) }})
}

pub async fn get(host: &HostClient, input: Selection) -> Result<Value> {
    let account = runtime(host, &input.account_id).await?;
    let (version, record) = load(host, &input.account_id).await?;
    Ok(view(version, &record, account.credential_revision))
}

pub async fn save(host: &HostClient, input: Save) -> Result<Value> {
    let account = runtime(host, &input.account_id).await?;
    if account.authentication_kind != "api_key" {
        return Err(ApiError::invalid("该账号不是 API Key 账号"));
    }
    let (current_version, current) = load(host, &input.account_id).await?;
    if current_version.unwrap_or(0) != input.expected_version {
        return Err(ApiError::conflict("查询设置或快照已更新，请重新读取"));
    }
    let record = if current.credential_revision == account.credential_revision
        && current.mode == input.query_type
    {
        current
    } else {
        Record {
            credential_revision: account.credential_revision,
            mode: input.query_type,
            ..Default::default()
        }
    };
    let version = state::put(
        host,
        "upstream_quota",
        &input.account_id,
        (input.expected_version != 0).then_some(input.expected_version),
        &record,
    )
    .await?;
    if account.enabled && record.mode != QuotaQueryMode::Disabled {
        return refresh_account(host, &input.account_id, true).await;
    }
    Ok(view(Some(version), &record, account.credential_revision))
}

pub async fn refresh(host: &HostClient, input: Selection) -> Result<Value> {
    refresh_account(host, &input.account_id, true).await
}

async fn refresh_account(host: &HostClient, id: &str, manual: bool) -> Result<Value> {
    let lock = {
        let mut locks = LOCKS.get_or_init(Default::default).lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(id).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(id.into(), Arc::downgrade(&lock));
            lock
        }
    };
    let _guard = lock.lock().await;
    let account = runtime(host, id).await?;
    let (version, mut record) = load(host, id).await?;
    if !account.enabled || record.mode == QuotaQueryMode::Disabled {
        return Ok(view(version, &record, account.credential_revision));
    }
    if account.authentication_kind != "api_key" {
        return Err(ApiError::invalid("该账号不是 API Key 账号"));
    }
    let interval = if manual {
        if matches!(
            record.mode,
            QuotaQueryMode::KimiCoding
                | QuotaQueryMode::KimiCodingGlobal
                | QuotaQueryMode::CommandCode
        ) {
            15
        } else {
            10
        }
    } else {
        record.mode.refresh_interval().as_secs() as i64
    };
    if record.credential_revision == account.credential_revision
        && record
            .last_attempt_at
            .is_some_and(|at| (now() - at).num_seconds() < interval)
    {
        return Ok(view(version, &record, account.credential_revision));
    }
    if record.credential_revision != account.credential_revision {
        record = Record {
            credential_revision: account.credential_revision,
            mode: record.mode,
            ..Default::default()
        };
    }
    record.last_attempt_at = Some(now());
    record.error = Some("refreshing".into());
    let claim = state::put(host, "upstream_quota", id, version, &record).await?;
    let credential: AuthCredential = host::binary(
        host,
        "host.auth.get",
        &AuthGetRequest {
            account_id: id.into(),
        },
    )
    .await?;
    if credential.credential_revision != record.credential_revision {
        return Err(ApiError::conflict("账号连接已变更，请重新获取额度"));
    }
    let result = network::fetch(host, &credential, record.mode)
        .await
        .and_then(|response| {
            // 私有状态单条上限为 256 KiB，给配置、时间和错误字段预留空间
            if serde_json::to_vec(&response)
                .map_err(|_| "invalid_response")?
                .len()
                > 240 * 1024
            {
                return Err("response_too_large");
            }
            Ok(response)
        });
    let current = runtime(host, id).await?;
    if current.credential_revision != credential.credential_revision || !current.enabled {
        return Err(ApiError::conflict("账号连接或启用状态已变更"));
    }
    match result {
        Ok(response) => {
            record.accounts = response.accounts;
            record.fetched_at = Some(now());
            record.error = None;
            record.blocked = false;
        }
        Err(code) => {
            record.error = Some(code.into());
            record.blocked |= matches!(code, "unauthorized" | "forbidden" | "sharing_disabled");
        }
    }
    let version = state::put(host, "upstream_quota", id, Some(claim), &record).await?;
    Ok(view(Some(version), &record, current.credential_revision))
}

pub async fn maintain(host: &HostClient) -> Result<()> {
    let mut accounts = directory(host)
        .await?
        .into_iter()
        .filter(|account| account.enabled && account.authentication_kind == "api_key");
    let mut pending = tokio::task::JoinSet::new();
    let mut failure = None;
    loop {
        while pending.len() < 4 {
            let Some(account) = accounts.next() else {
                break;
            };
            let host = host.clone();
            pending.spawn(async move { refresh_account(&host, &account.account_id, false).await });
        }
        let Some(result) = pending.join_next().await else {
            break;
        };
        match result {
            Ok(Ok(_)) => (),
            Ok(Err(error)) => failure = Some(error),
            Err(_) => {
                failure = Some(ApiError::new(
                    502,
                    "refresh_interrupted",
                    "后台额度任务中断",
                ))
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

pub(crate) async fn export_accounts(
    host: &HostClient,
    account: &AuthRuntimeAccount,
) -> Result<Vec<QuotaAccount>> {
    let (_, record) = load(host, &account.account_id).await?;
    if record.credential_revision != account.credential_revision
        || record.blocked
        || record.mode == QuotaQueryMode::Disabled
    {
        return Ok(vec![]);
    }
    let mut accounts = record.accounts;
    if record.error.is_some()
        || record
            .fetched_at
            .is_none_or(|at| (now() - at).num_seconds() >= 600)
    {
        for entry in &mut accounts {
            if entry.status == QuotaObservationStatus::Observed {
                entry.status = QuotaObservationStatus::Stale;
            }
        }
    }
    Ok(accounts)
}

pub(crate) async fn sharing_record(host: &HostClient, id: &str) -> Result<(Option<u64>, Sharing)> {
    let (version, value) = state::get(host, "quota_sharing", id).await?;
    Ok((version, value.unwrap_or_default()))
}

pub async fn sharing(host: &HostClient, input: SharingSelection) -> Result<Value> {
    host.key_facts(gateway_plugin_sdk::call::data::ClientKeyFactsQuery {
        client_key_id: input.client_key_id.clone(),
    })
    .await?;
    let (version, value) = sharing_record(host, &input.client_key_id).await?;
    Ok(sharing_view(version, &value))
}

pub async fn save_sharing(host: &HostClient, input: SaveSharing) -> Result<Value> {
    host.key_facts(gateway_plugin_sdk::call::data::ClientKeyFactsQuery {
        client_key_id: input.client_key_id.clone(),
    })
    .await?;
    let (version, current) = sharing_record(host, &input.client_key_id).await?;
    if version.unwrap_or(0) != input.expected_config_revision {
        return Err(ApiError::conflict("分享设置已变更，请重新读取"));
    }
    let value = Sharing {
        enabled: input.enabled,
        include_names: input.include_names,
        include_models: input.include_models,
        salt: if current.salt.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            current.salt
        },
    };
    let version = state::put(host, "quota_sharing", &input.client_key_id, version, &value).await?;
    Ok(sharing_view(Some(version), &value))
}

fn sharing_view(version: Option<u64>, value: &Sharing) -> Value {
    json!({"configRevision":version.unwrap_or(0),"enabled":value.enabled,"includeNames":value.include_names,"includeModels":value.include_models})
}

fn query_error(code: &str) -> &str {
    match code {
        "refreshing" => "上次查询未完成，等待后台重试",
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
