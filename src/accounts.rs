use crate::{
    api::{ApiError, Result},
    host,
    sources::{Candidate, ParsedSource, display_url, normalized_url},
    state,
};
use gateway_plugin_sdk::{
    ErrorCode,
    call::host::{
        AuthCredential, AuthGetRequest, AuthListRequest, AuthListResult, AuthSaveRequest,
        AuthSaveResult, CredentialFacts,
    },
    client::HostClient,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mapping {
    app_type: String,
    provider_id: String,
    installation_id: String,
    account_id: Option<String>,
    status: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mappings {
    entries: Vec<Mapping>,
}
struct Group {
    members: Vec<Candidate>,
    targets: Vec<AuthCredential>,
    unknown: bool,
}

async fn existing(host: &HostClient) -> Result<Vec<AuthCredential>> {
    let mut cursor = None;
    let mut result = Vec::new();
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
        for account in page.accounts {
            if account.authentication_kind == "api_key" {
                let credential = host::binary(
                    host,
                    "host.auth.get",
                    &AuthGetRequest {
                        account_id: account.account_id,
                    },
                )
                .await?;
                result.push(credential);
            }
        }
        if result.len() > 5000 {
            return Err(ApiError::invalid("宿主账号超过本次核对范围"));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(result)
}
async fn mapping_state(host: &HostClient) -> Result<(Option<u64>, Mappings)> {
    let (version, state) = state::get(host, "imports", "index").await?;
    Ok((version, state.unwrap_or_default()))
}
fn material(account: &AuthCredential) -> Option<(String, &str)> {
    let url = normalized_url(account.facts.material.get("base_url")?.as_str()?).ok()?;
    let key = account.facts.material.get("api_key")?.as_str()?;
    Some((url, key))
}
fn grouped(source: &ParsedSource, accounts: &[AuthCredential], mappings: &Mappings) -> Vec<Group> {
    let mut groups: BTreeMap<(String, String), Vec<Candidate>> = BTreeMap::new();
    for candidate in &source.candidates {
        if candidate.reason.is_none() {
            groups
                .entry((
                    candidate.base_url.clone().expect("兼容项有地址"),
                    candidate.api_key.clone().expect("兼容项有API密钥"),
                ))
                .or_default()
                .push(candidate.clone());
        }
    }
    groups
        .into_iter()
        .map(|((url, key), members)| {
            let related: Vec<_> = mappings
                .entries
                .iter()
                .filter(|mapping| {
                    members
                        .iter()
                        .any(|m| m.app == mapping.app_type && m.provider == mapping.provider_id)
                })
                .collect();
            let mut unknown = false;
            for mapping in &related {
                if mapping.status == "pending"
                    && !accounts.iter().any(|a| {
                        a.facts
                            .material
                            .get("installation_id")
                            .and_then(Value::as_str)
                            == Some(mapping.installation_id.as_str())
                    })
                {
                    unknown = true;
                }
            }
            let targets = accounts
                .iter()
                .filter(|account| {
                    material(account).is_some_and(|(u, k)| u == url && k == key)
                        || related.iter().any(|m| {
                            m.account_id.as_deref() == Some(account.account_id.as_str())
                                || account
                                    .facts
                                    .material
                                    .get("installation_id")
                                    .and_then(Value::as_str)
                                    == Some(m.installation_id.as_str())
                        })
                })
                .cloned()
                .collect();
            Group {
                members,
                targets,
                unknown,
            }
        })
        .collect()
}
pub async fn preview(host: &HostClient, source: &ParsedSource) -> Result<Value> {
    let accounts = existing(host).await?;
    let (_, mappings) = mapping_state(host).await?;
    let grouped = grouped(source, &accounts, &mappings);
    let group_ids: BTreeMap<_, _> = grouped
        .iter()
        .flat_map(|group| {
            group
                .members
                .iter()
                .map(|member| (member.id.as_str(), group.members[0].id.as_str()))
        })
        .collect();
    let groups: Vec<_> = grouped.iter().map(|group| {
        let first=&group.members[0];
        let mut choices=Vec::new();
        for member in &group.members { choices.push(json!({"id":member.id,"name":member.name,"kind":"source","accountId":null,"revision":null})); }
        for target in &group.targets { choices.push(json!({"id":format!("host:{}",target.account_id),"name":target.facts.name,"kind":"host","accountId":target.account_id,"revision":target.credential_revision})); }
        let targets: Vec<_> = group.targets.iter().map(|t|json!({"accountId":t.account_id,"revision":t.credential_revision})).collect();
        let sources: Vec<_> = group.members.iter().map(|m| json!({"appType":m.app,"providerId":m.provider,"protocol":m.protocol,"model":m.model,"endpoints":m.endpoints.iter().map(|u| display_url(u)).collect::<Vec<_>>()})).collect();
        json!({"id":first.id,"address":first.base_url,"choices":choices,"targets":targets,"sources":sources,"status":if group.unknown {"unknown"} else if group.targets.is_empty() {"create"} else {"replace"},"message":if group.unknown {"此前保存结果未知，未找到唯一回执，禁止再次创建"} else if group.targets.is_empty() {"新建账号"} else {"确认后替换选中账号，其他账号保持原样"}})
    }).collect();
    let rows: Vec<_> = source.candidates.iter().map(|candidate| {
        let group_id = group_ids.get(candidate.id.as_str());
        json!({"id":candidate.id,"groupId":group_id,"name":candidate.name,"appType":candidate.app,"address":candidate.base_url,"hasKey":candidate.api_key.is_some(),"reason":candidate.reason})
    }).collect();
    let unsupported: Vec<_> = source
        .candidates
        .iter()
        .filter(|m| m.reason.is_some())
        .map(|m| json!({"appType":m.app,"providerId":m.provider,"name":m.name,"reason":m.reason}))
        .collect();
    Ok(
        json!({"rows":rows,"groups":groups,"unsupported":unsupported,"note":"相同 API 与API密钥只接入一次，名称不同可在接入时选择"}),
    )
}

pub fn key(source: &ParsedSource, candidate_id: &str) -> Result<Value> {
    let candidate = source
        .candidates
        .iter()
        .find(|candidate| candidate.id == candidate_id)
        .ok_or_else(|| ApiError::new(404, "candidate", "来源项不存在，请重新预览"))?;
    let key = candidate
        .api_key
        .as_deref()
        .ok_or_else(|| ApiError::invalid("该来源未解析到有效API密钥"))?;
    Ok(json!({"key":key}))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub group_id: String,
    pub choice_id: String,
    pub expected_revision: Option<u64>,
    pub target_account_id: Option<String>,
}
fn mapped(
    mappings: &mut Mappings,
    members: &[Candidate],
    installation: &str,
    account: Option<&str>,
    status: &str,
) -> Result<()> {
    for member in members {
        let record = Mapping {
            app_type: member.app.clone(),
            provider_id: member.provider.clone(),
            installation_id: installation.into(),
            account_id: account.map(str::to_owned),
            status: status.into(),
        };
        if let Some(old) = mappings
            .entries
            .iter_mut()
            .find(|m| m.app_type == member.app && m.provider_id == member.provider)
        {
            *old = record;
        } else {
            mappings.entries.push(record);
        }
    }
    if mappings.entries.len() > 1000 || serde_json::to_vec(mappings)?.len() > 262144 {
        return Err(ApiError::invalid("导入映射已达到容量限制"));
    }
    Ok(())
}
pub async fn import(
    host: &HostClient,
    source: &ParsedSource,
    selections: Vec<Selection>,
) -> Result<Value> {
    if selections.is_empty() || selections.len() > 100 {
        return Err(ApiError::invalid("每次请选择 1 至 100 组"));
    }
    let mut unique = HashSet::new();
    if selections.iter().any(|s| !unique.insert(&s.group_id)) {
        return Err(ApiError::invalid("不能重复提交同一组"));
    }
    let accounts = existing(host).await?;
    let (_, mappings) = mapping_state(host).await?;
    let groups = grouped(source, &accounts, &mappings);
    let mut results = Vec::new();
    let mut stopped = false;
    for selection in selections {
        if stopped {
            results.push(json!({"groupId":selection.group_id,"status":"not_run","message":"前项结果未知，本项未提交"}));
            continue;
        }
        let result = match groups
            .iter()
            .find(|g| g.members[0].id == selection.group_id)
        {
            Some(group) => save_group(host, group, &selection).await,
            None => Err(ApiError::conflict("候选组已变化，请重新预览")),
        };
        let item = match result {
            Ok(value) => value,
            Err(error) => {
                json!({"groupId":selection.group_id,"status":"failed","message":error.message})
            }
        };
        let unknown = item
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|s| matches!(s, "unknown" | "mapping_pending"));
        results.push(item);
        stopped = unknown;
    }
    Ok(json!({"results":results}))
}
async fn save_group(host: &HostClient, group: &Group, selection: &Selection) -> Result<Value> {
    if group.unknown {
        return Err(ApiError::conflict(
            "此前创建结果未知，请在宿主核对账号，不得重复创建",
        ));
    }
    let member = group.members.iter().find(|m| m.id == selection.choice_id);
    let named_target = group
        .targets
        .iter()
        .find(|a| format!("host:{}", a.account_id) == selection.choice_id);
    let selected_target = match &selection.target_account_id {
        Some(id) => Some(
            group
                .targets
                .iter()
                .find(|target| &target.account_id == id)
                .ok_or_else(|| ApiError::conflict("选中的宿主账号已变化，请重新预览"))?,
        ),
        None => named_target,
    };
    let target = if let Some(target) = selected_target {
        Some(target)
    } else if member.is_some() && group.targets.len() <= 1 {
        group.targets.first()
    } else {
        return Err(ApiError::invalid(
            "存在多个匹配账号时，请选择一个具体宿主账号",
        ));
    };
    let first = &group.members[0];
    let name = member
        .map(|m| m.name.clone())
        .or_else(|| named_target.map(|a| a.facts.name.clone()))
        .ok_or_else(|| ApiError::invalid("请选择一个名称"))?;
    let (version, mut mappings) = mapping_state(host).await?;
    let installation = target
        .and_then(|t| t.facts.material.get("installation_id"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let material = json!({"schema_version":1,"installation_id":installation,"api_key":first.api_key,"base_url":first.base_url,"transport":target.and_then(|t|t.facts.material.get("transport")).cloned().unwrap_or(json!("http"))});
    let mut facts = match target {
        Some(target) => target.facts.clone(),
        None => CredentialFacts {
            name: name.clone(),
            authentication_kind: "api_key".into(),
            material: serde_json::Map::new(),
            email: None,
            upstream_user_id: None,
            upstream_account_id: None,
            plan_type: None,
            has_refresh_token: false,
            access_token_expires_at_ms: None,
            next_refresh_at_ms: None,
        },
    };
    facts.name = name.clone();
    facts.material = material.as_object().expect("凭据材料为对象").clone();
    if let Some(target) = target {
        if selection.expected_revision != Some(target.credential_revision) {
            return Err(ApiError::conflict("账号凭据版本已变化，请重新预览"));
        }
        if target.facts == facts {
            mapped(
                &mut mappings,
                &group.members,
                &installation,
                Some(&target.account_id),
                "saved",
            )?;
            state::put(host, "imports", "index", version, &mappings).await?;
            return Ok(
                json!({"groupId":selection.group_id,"status":"skipped","accountId":target.account_id,"message":"账号无变化，已核对来源映射"}),
            );
        }
    } else if selection.expected_revision.is_some() {
        return Err(ApiError::conflict("目标账号已变化，请重新预览"));
    }
    // 先登记结果未知的来源标识。进程中断后只能查回执，不能无条件再次创建。
    mapped(
        &mut mappings,
        &group.members,
        &installation,
        target.map(|t| t.account_id.as_str()),
        "pending",
    )?;
    let reserved = state::put(host, "imports", "index", version, &mappings).await?;
    let request = match target {
        Some(target) => AuthSaveRequest::Replace {
            account_id: target.account_id.clone(),
            credential_revision: target.credential_revision,
            facts,
        },
        None => AuthSaveRequest::Create {
            provider_id: "openai".into(),
            facts,
        },
    };
    let saved: std::result::Result<AuthSaveResult, _> =
        host::binary(host, "host.auth.save", &request).await;
    match saved {
        Ok(saved) => {
            mapped(
                &mut mappings,
                &group.members,
                &installation,
                Some(&saved.account_id),
                "saved",
            )?;
            match state::put(host, "imports", "index", Some(reserved), &mappings).await {
                Ok(_) => Ok(
                    json!({"groupId":selection.group_id,"status":"saved","accountId":saved.account_id,"message":"已保存账号和来源映射"}),
                ),
                Err(_) => Ok(
                    json!({"groupId":selection.group_id,"status":"mapping_pending","accountId":saved.account_id,"message":"账号已保存，映射保存未确认，请重新预览恢复"}),
                ),
            }
        }
        Err(error)
            if matches!(
                error.code,
                ErrorCode::InvalidInput | ErrorCode::Rejected | ErrorCode::Conflict
            ) =>
        {
            mapped(
                &mut mappings,
                &group.members,
                &installation,
                target.map(|t| t.account_id.as_str()),
                "failed",
            )?;
            state::put(host, "imports", "index", Some(reserved), &mappings).await?;
            Err(ApiError::from(error))
        }
        Err(_) => Ok(
            json!({"groupId":selection.group_id,"status":"unknown","message":"账号保存结果未知，请在宿主核对后重新预览"}),
        ),
    }
}
