//! 模型账号优先顺序由插件持有，宿主负责候选资格、账号租约和失败重试

pub mod logs;

use crate::{
    account_models,
    api::{ApiError, Result},
    state,
};
use gateway_plugin_sdk::{
    PluginFault,
    call::{
        data::{AccountFacts, AccountFactsQuery},
        policy::{AccountScheduleDecision, AccountScheduleRequest},
    },
    client::{HostClient, TypedCall, TypedReply},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use tokio::task::JoinSet;

const NAMESPACE: &str = "model_priority";
const KEY: &str = "rules";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    provider: String,
    model: String,
    enabled: bool,
    account_ids: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    rules: Vec<Rule>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    expected_version: Option<u64>,
    rule: Rule,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Delete {
    expected_version: Option<u64>,
    provider: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lookup {
    provider: String,
    model: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    id: String,
    provider: String,
    name: String,
    enabled: bool,
}

impl From<AccountFacts> for Account {
    fn from(value: AccountFacts) -> Self {
        Self {
            id: value.account_id,
            provider: value.provider_id,
            name: value.name,
            enabled: value.enabled,
        }
    }
}

fn valid_text(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn validate_target(provider: &str, model: &str) -> Result<()> {
    if !valid_text(provider, 128) || !valid_text(model, 512) {
        return Err(ApiError::invalid(
            "Provider 或模型名称无效，请填写完整模型 ID",
        ));
    }
    Ok(())
}

async fn load(host: &HostClient) -> std::result::Result<(Option<u64>, Configuration), PluginFault> {
    let (version, value) = state::get(host, NAMESPACE, KEY).await?;
    Ok((version, value.unwrap_or_default()))
}

async fn accounts(host: &HostClient, provider: Option<String>) -> Result<Vec<Account>> {
    let mut cursor = None;
    let mut accounts = Vec::new();
    loop {
        let page = host
            .account_facts(AccountFactsQuery {
                provider_id: provider.clone(),
                cursor,
                limit: 200,
            })
            .await?;
        accounts.extend(page.accounts.into_iter().map(Account::from));
        if accounts.len() > 5000 {
            return Err(ApiError::invalid("账号目录超过 5000 项，请缩小账号范围"));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Ok(accounts),
        }
    }
}

pub async fn get(host: &HostClient) -> Result<Value> {
    let (version, value) = load(host).await?;
    let accounts = accounts(host, None).await?;
    Ok(json!({"version": version, "rules": value.rules, "accounts": accounts}))
}

fn require_version(current: Option<u64>, expected: Option<u64>) -> Result<()> {
    if current != expected {
        return Err(ApiError::conflict("优先级规则已更新，请重新读取后修改"));
    }
    Ok(())
}

pub async fn save(host: &HostClient, input: Save) -> Result<Value> {
    validate_target(&input.rule.provider, &input.rule.model)?;
    let ids = &input.rule.account_ids;
    if ids.is_empty()
        || ids.len() > 1000
        || ids.iter().any(|id| !valid_text(id, 256))
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err(ApiError::invalid("请选择 1–1000 个不重复的上游账号"));
    }
    let (version, mut value) = load(host).await?;
    require_version(version, input.expected_version)?;
    if let Some(rule) = value
        .rules
        .iter_mut()
        .find(|rule| rule.provider == input.rule.provider && rule.model == input.rule.model)
    {
        *rule = input.rule;
    } else {
        if value.rules.len() >= 100 {
            return Err(ApiError::invalid("最多保存 100 条模型优先级规则"));
        }
        value.rules.push(input.rule);
    }
    if serde_json::to_vec(&value)?.len() > 240 * 1024 {
        return Err(ApiError::invalid(
            "优先级规则超过存储容量，请减少规则或账号数量",
        ));
    }
    let version = state::put(host, NAMESPACE, KEY, version, &value).await?;
    Ok(json!({"version": version, "rules": value.rules}))
}

pub async fn delete(host: &HostClient, input: Delete) -> Result<Value> {
    validate_target(&input.provider, &input.model)?;
    let (version, mut value) = load(host).await?;
    require_version(version, input.expected_version)?;
    value
        .rules
        .retain(|rule| rule.provider != input.provider || rule.model != input.model);
    let version = state::put(host, NAMESPACE, KEY, version, &value).await?;
    Ok(json!({"version": version, "rules": value.rules}))
}

async fn has_model(host: &HostClient, account_id: &str, model: &str) -> Result<bool> {
    Ok(account_models::list(host, account_id)
        .await?
        .iter()
        .any(|id| id == model))
}

pub async fn lookup(host: &HostClient, input: Lookup) -> Result<Value> {
    validate_target(&input.provider, &input.model)?;
    let accounts = accounts(host, Some(input.provider.clone())).await?;
    // 单账号目录失败单独返回，整体查询受期限限制
    tokio::time::timeout(std::time::Duration::from_secs(25), async {
        let mut pending = accounts.into_iter();
        let mut tasks = JoinSet::new();
        let mut matched = Vec::new();
        let mut failures = Vec::new();
        loop {
            while tasks.len() < 4 {
                let Some(account) = pending.next() else { break };
                let host = host.clone();
                let model = input.model.clone();
                tasks.spawn(async move {
                    let result = has_model(&host, &account.id, &model).await;
                    (account, result)
                });
            }
            let Some(result) = tasks.join_next().await else {
                break;
            };
            let (account, result) = result.map_err(|_| {
                ApiError::new(500, "model_catalog", "模型目录查询任务未完成，请重新获取")
            })?;
            match result {
                Ok(true) => matched.push(account),
                Ok(false) => {}
                Err(error) => failures.push(
                    json!({"id": account.id, "name": account.name, "message": error.message}),
                ),
            }
        }
        matched.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(json!({"accounts": matched, "failures": failures}))
    })
    .await
    .map_err(|_| {
        ApiError::new(
            504,
            "model_catalog_timeout",
            "查询超过 25 秒，请先在账号管理刷新模型目录后重试",
        )
    })?
}

pub async fn schedule(
    logs: &logs::Logs,
    call: TypedCall<AccountScheduleRequest>,
) -> std::result::Result<TypedReply<AccountScheduleDecision>, PluginFault> {
    let request = &call.request;
    let (_, value) = match load(&call.host).await {
        Ok(value) => value,
        Err(error) => {
            logs.record(
                request,
                logs::Decision {
                    attempt_index: request.attempt_index,
                    kind: logs::DecisionKind::RulesError,
                    primary_account_id: None,
                    selected_account_id: None,
                },
            )
            .await;
            return Err(error);
        }
    };
    let rule = value.rules.iter().find(|rule| {
        rule.enabled
            && rule.provider == request.provider
            && Some(rule.model.as_str()) == request.model.as_deref()
    });
    if let Some(rule) = rule {
        let available: HashSet<_> = request
            .candidates
            .iter()
            .filter(|candidate| candidate.available)
            .map(|candidate| candidate.account_id.as_str())
            .collect();
        for (index, id) in rule.account_ids.iter().enumerate() {
            if available.contains(id.as_str()) {
                logs.record(
                    request,
                    logs::Decision {
                        attempt_index: request.attempt_index,
                        kind: if index == 0 {
                            logs::DecisionKind::Primary
                        } else {
                            logs::DecisionKind::Backup
                        },
                        primary_account_id: rule.account_ids.first().cloned(),
                        selected_account_id: Some(id.clone()),
                    },
                )
                .await;
                return Ok(TypedReply::new(AccountScheduleDecision::Pick {
                    account_id: id.clone(),
                }));
            }
        }
    }
    logs.record(
        request,
        logs::Decision {
            attempt_index: request.attempt_index,
            kind: if rule.is_some() {
                logs::DecisionKind::NativeFallback
            } else {
                logs::DecisionKind::Unmatched
            },
            primary_account_id: rule.and_then(|rule| rule.account_ids.first().cloned()),
            selected_account_id: None,
        },
    )
    .await;
    Ok(TypedReply::new(AccountScheduleDecision::Delegate))
}
