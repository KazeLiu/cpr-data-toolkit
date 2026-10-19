use crate::{
    api::{ApiError, Result},
    usage,
};
use gateway_plugin_sdk::{call::services::usage::*, client::HostClient};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncPage {
    pub start_ms: i64,
    pub end_ms: i64,
    pub provider: String,
    pub mode: String,
    pub cursor: Option<String>,
}
pub async fn sync_page(host: &HostClient, input: SyncPage) -> Result<Value> {
    if !matches!(input.mode.as_str(), "missing" | "all")
        || !matches!(input.provider.as_str(), "openai" | "xai")
        || input.start_ms >= input.end_ms
    {
        return Err(ApiError::invalid(
            "请选择有效的费用同步范围、Provider 和方式",
        ));
    }
    let mut filter = usage::filter(input.start_ms, input.end_ms);
    filter.provider = Some(input.provider);
    let page = host
        .service::<ListRecords>(ListUsageRecords {
            filter,
            cursor: input.cursor,
            limit: Some(50),
        })
        .await?;
    let mut results = Vec::new();
    let mut items = Vec::new();
    for record in &page.items {
        let data = &record.data;
        let reason = if matches!(data.outcome, UsageOutcome::Running) {
            Some("请求仍在运行")
        } else if input.mode == "missing" && !matches!(data.cost, UsageCost::Unavailable) {
            Some("已有费用")
        } else if data.provider.is_none() || data.model.is_none() {
            Some("Provider 或模型缺失")
        } else {
            None
        };
        if let Some(reason) = reason {
            results.push(json!({"id":record.id,"status":"skipped","message":reason}));
        } else {
            items.push(UsagePricingInput {
                id: record.id.clone(),
                provider: data.provider.clone().unwrap(),
                model: data.model.clone().unwrap(),
                tokens: data.tokens.clone(),
                service_tier: data.service_tier.clone(),
                price: UsagePriceSelection::Current,
            });
        }
    }
    let mut changes = Vec::new();
    if !items.is_empty() {
        let estimate = host
            .service::<EvaluatePricing>(EvaluateUsagePricing { items })
            .await?;
        for item in estimate.items {
            match item {
                UsagePricingResult::Estimated { id, cost, .. } => {
                    results.push(json!({"id":id,"status":"saved","message":"已按当前 Token 价目写回估算费用"}));
                    changes.push(UsageRecordChange::Update {
                        id,
                        tokens: None,
                        cost: Some(cost),
                    });
                }
                UsagePricingResult::NotEstimable { id, reason } => {
                    results.push(json!({"id":id,"status":"skipped","message":reason}));
                }
            }
        }
    }
    if !changes.is_empty() {
        // 宿主单批事务失败时整批回滚；响应丢失后保留当前游标，由管理员确认后继续
        host.service::<WriteRecords>(WriteUsageRecords { changes })
            .await?;
    }
    Ok(json!({"scanned":page.items.len(),"results":results,"nextCursor":page.next_cursor}))
}
