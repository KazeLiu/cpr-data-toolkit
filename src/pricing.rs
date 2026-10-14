use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    call::{
        host::HttpRequest,
        services::settings::{
            self, ModelPriceOverride, MutationActor, MutationContext, PricingChange, UpdatePricing,
        },
    },
    client::HostClient,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashSet, str::FromStr};

pub fn actor(request_id: String) -> MutationContext {
    MutationContext {
        actor: MutationActor::System,
        request_id,
    }
}
fn service_error(error: gateway_plugin_sdk::call::services::ServiceError) -> ApiError {
    ApiError::new(
        if error.kind == "conflict" { 409 } else { 502 },
        "pricing",
        "定价操作未完成，请重新读取价目核对",
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Update {
    pub provider: String,
    pub models: Vec<String>,
    pub action: String,
    pub pricing: Option<ModelPriceOverride>,
    pub multiplier_bps: Option<u32>,
}
impl Update {
    fn command(self) -> Result<UpdatePricing> {
        if !matches!(self.provider.as_str(), "openai" | "xai")
            || self.models.is_empty()
            || self.models.len() > 500
            || self.models.iter().any(|m| {
                m.is_empty()
                    || m.len() > 128
                    || m.chars().any(char::is_whitespace)
                    || m.chars().any(char::is_control)
            })
        {
            return Err(ApiError::invalid("Provider、模型 ID 或批量数量无效"));
        }
        let change = match self.action.as_str() {
            "replace" => {
                let price = self
                    .pricing
                    .ok_or_else(|| ApiError::invalid("请填写价格"))?;
                if price.multiplier_bps > 1_000_000 || price.bands.is_empty() {
                    return Err(ApiError::invalid("价格档位或倍率无效"));
                }
                for band in price.bands.values() {
                    for value in [
                        &band.input,
                        &band.output,
                        &band.cache_read,
                        &band.cache_write,
                    ] {
                        let decimal = Decimal::from_str(value)
                            .map_err(|_| ApiError::invalid("价格须为十进制数字"))?;
                        if decimal.is_sign_negative() {
                            return Err(ApiError::invalid("价格不能为负数"));
                        }
                    }
                }
                PricingChange::Replace(price)
            }
            "reset" => PricingChange::Reset,
            "delete" => PricingChange::Delete,
            "multiplier" => {
                let value = self
                    .multiplier_bps
                    .filter(|v| *v <= 1_000_000)
                    .ok_or_else(|| ApiError::invalid("倍率无效"))?;
                PricingChange::Multiplier(value)
            }
            _ => return Err(ApiError::invalid("未知定价操作")),
        };
        Ok(UpdatePricing {
            provider: self.provider,
            models: self.models,
            change,
        })
    }
}
pub async fn update(host: &HostClient, request_id: String, requests: Vec<Update>) -> Result<Value> {
    if requests.is_empty() || requests.len() > 500 {
        return Err(ApiError::invalid("请选择 1 至 500 项操作"));
    }
    let commands = requests
        .into_iter()
        .map(Update::command)
        .collect::<Result<Vec<_>>>()?;
    let mut identities = HashSet::new();
    for command in &commands {
        if command
            .models
            .iter()
            .any(|model| !identities.insert((command.provider.clone(), model.clone())))
        {
            return Err(ApiError::invalid(
                "同一批次不能重复修改同一 Provider 的同一个模型",
            ));
        }
    }
    let mut results = Vec::new();
    let mut stopped = false;
    for command in commands {
        let models = command.models.clone();
        if stopped {
            results.push(
                json!({"models":models,"status":"not_run","message":"前项结果未知，本项未提交"}),
            );
            continue;
        }
        let result = host
            .service::<settings::Update>((actor(request_id.clone()), command))
            .await;
        let (status, message) = match result {
            Ok(()) => ("saved", "已保存"),
            Err(error)
                if matches!(
                    error.kind.as_str(),
                    "invalid_input" | "invalid" | "conflict" | "rejected"
                ) =>
            {
                ("failed", "宿主拒绝此项，请核对输入与当前价目")
            }
            Err(_) => ("unknown", "结果未知，请重新读取价目核对"),
        };
        results.push(json!({"models":models,"status":status,"message":message}));
        stopped = status == "unknown";
    }
    Ok(json!({"results":results}))
}
pub async fn catalog(host: &HostClient) -> Result<Value> {
    Ok(serde_json::to_value(
        host.service::<settings::Pricing>(())
            .await
            .map_err(service_error)?,
    )?)
}
pub async fn preview_sync(host: &HostClient) -> Result<Value> {
    Ok(serde_json::to_value(
        host.service::<settings::PreviewPricingSync>(())
            .await
            .map_err(service_error)?,
    )?)
}
pub async fn sync(
    host: &HostClient,
    request_id: String,
    command: settings::SyncPricing,
) -> Result<Value> {
    host.service::<settings::Sync>((actor(request_id), command))
        .await
        .map_err(service_error)?;
    Ok(json!({"saved":true}))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommunityPrice {
    source_provider: String,
    model: String,
    name: String,
    input: String,
    output: String,
    cache_read: String,
    cache_write: String,
}
fn amount(value: Option<&Value>) -> Option<String> {
    let text = match value? {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        _ => return None,
    };
    let decimal = Decimal::from_str(&text).ok()?;
    (!decimal.is_sign_negative()).then(|| decimal.normalize().to_string())
}
fn cache_amount(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => Some("0".into()),
        Some(value) => amount(Some(value)),
    }
}
pub async fn community(
    host: &HostClient,
    provider_id: &str,
    offset: usize,
    search: &str,
    model_filter: &str,
) -> Result<Value> {
    if search.len() > 1024 || model_filter.len() > 1024 {
        return Err(ApiError::invalid("社区价目筛选条件过长"));
    }
    let search = search.trim().to_lowercase();
    let model_filter = model_filter.trim().to_lowercase();
    let mut response = host
        .http(
            HttpRequest {
                method: "GET".into(),
                url: "https://models.dev/api.json".into(),
                headers: vec![],
            },
            vec![],
        )
        .await?;
    if response.status != 200 {
        response.body.close().await?;
        return Err(ApiError::new(502, "community", "models.dev 价目暂不可用"));
    }
    let bytes = response.body.collect(16 * 1024 * 1024).await?;
    let catalog: Value = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::new(502, "community", "models.dev 价目格式无效"))?;
    let providers = catalog
        .as_object()
        .ok_or_else(|| ApiError::new(502, "community", "models.dev 价目格式无效"))?;
    let provider_options: Vec<_> = providers
        .iter()
        .map(|(id, p)| json!({"id":id,"name":p.get("name").and_then(Value::as_str).unwrap_or(id)}))
        .collect();
    let selected = providers
        .get(provider_id)
        .ok_or_else(|| ApiError::invalid("models.dev 来源供应商不存在"))?;
    let mut rows = Vec::new();
    let mut skipped = 0;
    for (provider, value) in [(provider_id, selected)] {
        let Some(models) = value.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (id, model) in models {
            let cost = model.get("cost");
            let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
            if !id.to_lowercase().contains(&model_filter) {
                continue;
            }
            let matches_search = search.is_empty()
                || id.to_lowercase().contains(&search)
                || name.to_lowercase().contains(&search)
                || ["input", "output", "cache_read", "cache_write"]
                    .iter()
                    .any(|field| {
                        amount(cost.and_then(|value| value.get(field)))
                            .is_some_and(|amount| amount.contains(&search))
                    });
            if !matches_search {
                continue;
            }
            let modalities = model
                .pointer("/modalities/output")
                .and_then(Value::as_array);
            if modalities.is_none_or(|v| v.iter().any(|m| m.as_str() != Some("text")))
                || cost.and_then(Value::as_object).is_some_and(|c| {
                    c.keys().any(|key| {
                        !matches!(
                            key.as_str(),
                            "input" | "output" | "cache_read" | "cache_write"
                        )
                    })
                })
            {
                skipped += 1;
                continue;
            }
            let (Some(input), Some(output)) = (
                amount(cost.and_then(|c| c.get("input"))),
                amount(cost.and_then(|c| c.get("output"))),
            ) else {
                skipped += 1;
                continue;
            };
            let (Some(cache_read), Some(cache_write)) = (
                cache_amount(cost.and_then(|c| c.get("cache_read"))),
                cache_amount(cost.and_then(|c| c.get("cache_write"))),
            ) else {
                skipped += 1;
                continue;
            };
            rows.push(CommunityPrice {
                source_provider: provider.into(),
                model: id.clone(),
                name: name.into(),
                input,
                output,
                cache_read,
                cache_write,
            });
        }
    }
    let total = rows.len();
    let rows: Vec<_> = rows.into_iter().skip(offset).take(100).collect();
    Ok(
        json!({"providers":provider_options,"rows":rows,"total":total,"skipped":skipped,"currency":"USD","unit":"每百万 Token"}),
    )
}
