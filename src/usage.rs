//! CCS 统计贡献的预览、范围核对与分批写入，以及手动统计修正

mod duplicates;
mod selection;
pub use selection::ImportOptions;

use crate::{
    api::{ApiError, Result},
    sources::ParsedSource,
    state,
};
use chrono::DateTime;
use gateway_plugin_sdk::{
    call::{middleware::http::Version, services::usage::*},
    client::{HostClient, HttpBody, HttpFrame, HttpRequest},
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use uuid::Uuid;

const BATCH: usize = 100;
const SOURCE_PREFIX: &str = "kazeliu.cpr-data-toolkit:ccs:";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportStep {
    pub task_id: String,
    pub offset: usize,
    pub options: ImportOptions,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskId {
    pub task_id: String,
}
struct ImportJob {
    id: String,
    changes: Vec<UsageAggregateChange>,
    offset: usize,
    touched: Instant,
    imported: usize,
    removed: usize,
    updated: usize,
    preserved: usize,
    options: ImportOptions,
    preview: crate::sources::UsagePreview,
    evidence: Vec<duplicates::Evidence>,
}
impl ImportJob {
    fn progress(&self) -> Value {
        json!({"taskId":self.id,"offset":self.offset,"total":self.changes.len(),
            "imported":self.imported,"removed":self.removed,"updated":self.updated,
            "preserved":self.preserved,"filtered":self.evidence.len(),
            "excluded":self.evidence.iter().filter(|item| item.excluded).count(),
            "uncertain":self.evidence.iter().filter(|item| !item.excluded).count(),
            "options":self.options,"done":self.offset == self.changes.len()})
    }
}
#[derive(Default)]
pub struct Imports(Mutex<Option<ImportJob>>);

pub fn filter(start_ms: i64, end_ms: i64) -> UsageRecordFilter {
    UsageRecordFilter {
        start_ms,
        end_ms,
        provider: None,
        model: None,
        account_ref: None,
        client_key_ref: None,
    }
}
fn valid_text(value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(ApiError::invalid("来源身份或模型为空、过长或包含控制字符"));
    }
    Ok(())
}
fn counts(
    tokens: [Option<i64>; 4],
    requests: i64,
    successes: i64,
) -> Result<BTreeMap<UsageMetric, UsageDelta>> {
    let [fresh, output, read, write] = tokens;
    let mut counts = BTreeMap::from([
        (UsageMetric::RequestCount, UsageDelta(requests)),
        (UsageMetric::SuccessCount, UsageDelta(successes)),
        (UsageMetric::FailureCount, UsageDelta(requests - successes)),
        (UsageMetric::CompletedRequestCount, UsageDelta(requests)),
    ]);
    let inclusive = match (fresh, read, write) {
        (Some(a), Some(b), Some(c)) => Some(
            a.checked_add(b)
                .and_then(|v| v.checked_add(c))
                .ok_or_else(|| ApiError::invalid("来源 Token 超出宿主整数范围"))?,
        ),
        _ => None,
    };
    let total = match (inclusive, output) {
        (Some(a), Some(b)) => Some(
            a.checked_add(b)
                .ok_or_else(|| ApiError::invalid("来源 Token 总量超出宿主整数范围"))?,
        ),
        _ => None,
    };
    for (metric, value) in [
        (UsageMetric::InputTokens, inclusive),
        (UsageMetric::OutputTokens, output),
        (UsageMetric::CachedInputTokens, read),
        (UsageMetric::CacheWriteInputTokens, write),
        (UsageMetric::TotalTokens, total),
    ] {
        if let Some(value) = value {
            if value < 0 {
                return Err(ApiError::invalid("来源 Token 不能为负数"));
            }
            counts.insert(metric, UsageDelta(value));
        }
    }
    Ok(counts)
}

fn aggregates(
    source: &crate::sources::UsagePreview,
    options: &ImportOptions,
    selection: &selection::Selection,
) -> Result<Vec<UsageAggregate>> {
    if options.source_name.is_empty()
        || options.source_name.len() > 32
        || !options
            .source_name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'-' | b'_'))
    {
        return Err(ApiError::invalid(
            "来源标识须为 1～32 个英文字母、数字、下划线或短横线",
        ));
    }
    let mut daily = HashSet::new();
    for row in source.rows.iter().filter(|row| !row.internal_duplicate) {
        if let Some(date) = &row.date {
            daily.insert((date.clone(), row.app.clone(), row.provider.clone()));
        }
    }
    let mut identities = HashSet::new();
    source
        .rows
        .iter()
        .filter(|row| !row.internal_duplicate)
        .map(|row| {
            valid_text(&row.app, 64)?;
            valid_text(&row.provider, 256)?;
            let bucket_start_ms = if let Some(date) = &row.date {
                selection.day(date)?.0
            } else {
                let time = DateTime::from_timestamp(
                    row.time
                        .ok_or_else(|| ApiError::invalid("来源请求时间缺失"))?,
                    0,
                )
                .ok_or_else(|| ApiError::invalid("来源请求时间无效"))?;
                let day = time
                    .with_timezone(&selection.timezone)
                    .date_naive()
                    .to_string();
                if daily.contains(&(day, row.app.clone(), row.provider.clone())) {
                    return Err(ApiError::invalid(
                        "同一来源供应商的明细与日聚合日期交叠，请先在来源侧确认覆盖范围后重新导出",
                    ));
                }
                time.timestamp_millis()
            };
            let model = if row.model.is_empty() || row.model.eq_ignore_ascii_case("unknown") {
                None
            } else {
                valid_text(&row.model, 128)?;
                Some(row.model.clone())
            };
            let key = record_key(options, row);
            let identity = UsageAggregateKey {
                bucket_start_ms,
                dimensions: UsageDimensions {
                    model,
                    ..Default::default()
                },
                key,
            };
            if !identities.insert(serde_json::to_string(&identity)?) {
                return Err(ApiError::invalid("来源存在重复用量身份，未提交导入"));
            }
            let mut costs = BTreeMap::new();
            if let Some(cost) = &row.cost {
                let amount: Decimal = cost
                    .parse()
                    .map_err(|_| ApiError::invalid("来源费用无效"))?;
                if amount.is_sign_negative() {
                    return Err(ApiError::invalid("来源费用不能为负数"));
                }
                costs.insert("USD".into(), amount.normalize().to_string());
            }
            Ok(UsageAggregate {
                identity,
                counts: counts(row.tokens, row.requests, row.successes)?,
                costs,
            })
        })
        .collect()
}

fn record_key(options: &ImportOptions, row: &crate::sources::UsageRow) -> String {
    format!(
        "{SOURCE_PREFIX}{}:{}",
        options.source_name,
        Uuid::new_v5(&Uuid::NAMESPACE_URL, row.identity.as_bytes())
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewPage {
    pub options: ImportOptions,
    pub offset: usize,
    pub review_offset: usize,
    pub task_id: Option<String>,
}

fn preview_page(
    preview: &crate::sources::UsagePreview,
    evidence: &[duplicates::Evidence],
    request: &PreviewPage,
) -> Value {
    let requests = preview
        .groups
        .iter()
        .try_fold(0u128, |sum, group| {
            sum.checked_add(group.requests.parse::<u128>().unwrap())
        })
        .map(|value| value.to_string());
    let tokens = preview
        .groups
        .iter()
        .try_fold(0u128, |sum, group| {
            sum.checked_add(group.total_tokens.as_ref()?.parse::<u128>().unwrap())
        })
        .map(|value| value.to_string());
    let cost = preview
        .groups
        .iter()
        .try_fold(Decimal::ZERO, |sum, group| {
            sum.checked_add(
                group
                    .original_cost_usd
                    .as_ref()?
                    .parse::<Decimal>()
                    .unwrap(),
            )
        })
        .map(|value| value.normalize().to_string());
    json!({"groups":preview.groups.iter().skip(request.offset).take(100).collect::<Vec<_>>(),
        "total":preview.groups.len(),"detailRows":preview.detail_rows,"excludedDuplicates":preview.excluded_duplicates,
        "rollupRows":preview.rollup_rows,"warnings":preview.warnings,
        "evidence":evidence.iter().skip(request.review_offset).take(100).collect::<Vec<_>>(),"evidenceTotal":evidence.len(),
        "totals":{"requests":requests,"tokens":tokens,"costUsd":cost}})
}

impl Imports {
    pub async fn preview(
        &self,
        source: Option<&ParsedSource>,
        request: PreviewPage,
    ) -> Result<Value> {
        if let Some(id) = &request.task_id {
            let mut current = self.0.lock().await;
            let job = current
                .as_mut()
                .filter(|job| {
                    &job.id == id
                        && job.options == request.options
                        && job.touched.elapsed() < Duration::from_secs(15 * 60)
                })
                .ok_or_else(|| ApiError::conflict("预览已失效，请按当前筛选重新预览"))?;
            job.touched = Instant::now();
            return Ok(preview_page(&job.preview, &job.evidence, &request));
        }
        let preview = selection::Selection::new(&request.options)?.select(source.unwrap())?;
        Ok(preview_page(&preview, &[], &request))
    }
    pub async fn expire(&self) {
        let mut current = self.0.lock().await;
        if current
            .as_ref()
            .is_some_and(|job| job.touched.elapsed() >= Duration::from_secs(15 * 60))
        {
            *current = None;
        }
    }
    pub async fn prepare(
        &self,
        host: &HostClient,
        source: &ParsedSource,
        options: ImportOptions,
    ) -> Result<Value> {
        let mut current = self.0.lock().await;
        if current.as_ref().is_some_and(|job| {
            job.offset > 0
                && job.offset < job.changes.len()
                && job.touched.elapsed() < Duration::from_secs(15 * 60)
        }) {
            return Err(ApiError::conflict(
                "已有未完成的用量导入，请先继续该任务，或等待闲置 15 分钟后重新预览",
            ));
        }
        let selection = selection::Selection::new(&options)?;
        let mut preview = selection.select(source)?;
        let values = aggregates(&preview, &options, &selection)?;
        if values.is_empty() {
            return Err(ApiError::invalid(
                "没有符合筛选条件的用量，不执行导入或清理",
            ));
        }
        let effective: Vec<_> = preview
            .rows
            .iter()
            .filter(|row| !row.internal_duplicate)
            .collect();
        let evidence = duplicates::inspect(
            host,
            &effective,
            &source.usage.rows,
            selection.start_ms,
            selection.end_ms,
        )
        .await?;
        let excluded: HashSet<_> = effective
            .iter()
            .zip(&evidence)
            .filter(|(_, report)| report.excluded)
            .map(|(row, _)| record_key(&options, row))
            .collect();
        let mut desired: HashMap<_, _> = values
            .into_iter()
            .filter(|value| !excluded.contains(&value.identity.key))
            .map(|value| (value.identity.key.clone(), value))
            .collect();
        let imported = desired.len();
        let mut changes = Vec::new();
        let mut updated = 0;
        let mut preserved = 0;
        let prefix = format!("{SOURCE_PREFIX}{}:", options.source_name);
        let mut cursor = None;
        let mut scanned = 0;
        loop {
            let page = host
                .service::<ListAggregates>(ListUsageAggregates {
                    filter: filter(-62135596800000, 253402300800000),
                    cursor,
                    limit: Some(200),
                })
                .await?;
            scanned += page.items.len();
            if scanned > 200000 {
                return Err(ApiError::invalid("插件统计贡献超过当前 200000 项核对容量"));
            }
            for old in page.items {
                if old.identity.key.starts_with(&prefix) {
                    let key = &old.identity.key;
                    // 覆盖按宿主贡献归属时间划定，不受本次导入的应用与模型筛选限制
                    if !selection.contains(old.identity.bucket_start_ms, None) {
                        if desired.get(key).is_some_and(|value| value != &old) {
                            return Err(ApiError::conflict(
                                "选中记录已有范围外贡献，请扩大筛选范围后预览，避免重复累计",
                            ));
                        }
                        preserved += 1;
                        continue;
                    }
                    if desired.get(key) == Some(&old) {
                        desired.remove(key);
                    } else if let Some(value) = desired.get(key) {
                        updated += 1;
                        if value.identity != old.identity {
                            changes.push(UsageAggregateChange::Delete {
                                identity: old.identity,
                            });
                        }
                    } else {
                        changes.push(UsageAggregateChange::Delete {
                            identity: old.identity,
                        });
                    }
                }
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let removed = changes.len();
        let mut puts: Vec<_> = desired.into_iter().collect();
        puts.sort_by(|a, b| a.0.cmp(&b.0));
        changes.extend(
            puts.into_iter()
                .map(|(_, aggregate)| UsageAggregateChange::Put { aggregate }),
        );
        preview.warnings.push("CPR 按所选开始时间范围查询，未填边界时使用选中记录的最早／最晚时间，并向前后扩展 30 秒；比较使用实际完成时间，范围外更早开始的长请求不推断重复".into());
        if preserved > 0 {
            preview.warnings.push(format!(
                "保留 {preserved} 项所选时间范围外的当前 CCS 来源贡献"
            ));
        }
        // 确认后只使用变更计划与展示快照，不再解析或筛选来源
        preview.rows.clear();
        let job = ImportJob {
            id: Uuid::new_v4().to_string(),
            changes,
            offset: 0,
            touched: Instant::now(),
            imported,
            removed,
            updated,
            preserved,
            options,
            preview,
            evidence,
        };
        let result = job.progress();
        *current = Some(job);
        Ok(result)
    }
    pub async fn apply(&self, host: &HostClient, step: ImportStep) -> Result<Value> {
        let mut current = self.0.lock().await;
        let job = current
            .as_mut()
            .filter(|job| {
                job.id == step.task_id && job.touched.elapsed() < Duration::from_secs(15 * 60)
            })
            .ok_or_else(|| {
                ApiError::conflict("导入任务不存在或已过期，请使用同一来源标识重新预览完整导出")
            })?;
        if job.options != step.options {
            return Err(ApiError::conflict(
                "来源或筛选已变化，不能继续旧预览，请重新预览",
            ));
        }
        job.touched = Instant::now();
        if step.offset > job.offset {
            return Err(ApiError::conflict("导入进度不一致，请保留任务后继续"));
        }
        if step.offset == job.offset && job.offset < job.changes.len() {
            let end = (job.offset + BATCH).min(job.changes.len());
            let input = WriteUsageAggregates {
                changes: job.changes[job.offset..end].to_vec(),
            };
            host.service::<WriteAggregates>(input).await?;
            job.offset = end;
        }
        Ok(job.progress())
    }
    pub async fn discard(&self, task: TaskId) -> Result<Value> {
        let mut current = self.0.lock().await;
        if current.as_ref().is_some_and(|job| job.id != task.task_id) {
            return Err(ApiError::conflict("当前任务已变化，请重新打开页面核对"));
        }
        *current = None;
        Ok(json!({"cleared":true}))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Adjustment {
    pub id: String,
    pub at_ms: i64,
    pub tokens: UsageDelta,
    pub requests: UsageDelta,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdjustmentState {
    command: Adjustment,
    status: String,
}
pub async fn pending_adjustment(host: &HostClient) -> Result<Value> {
    let (_, last) = state::get::<AdjustmentState>(host, "usage_adjustment", "last").await?;
    Ok(json!({"pending":last.filter(|last| last.status == "pending").map(|last| last.command)}))
}
pub async fn adjust(host: &HostClient, command: Adjustment) -> Result<Value> {
    Uuid::parse_str(&command.id).map_err(|_| ApiError::invalid("修正操作 ID 无效"))?;
    if DateTime::from_timestamp_millis(command.at_ms).is_none()
        || (command.tokens.get() == 0 && command.requests.get() == 0)
    {
        return Err(ApiError::invalid("归属时间或增减量无效"));
    }
    let (mut version, last) =
        state::get::<AdjustmentState>(host, "usage_adjustment", "last").await?;
    if let Some(last) = &last {
        if last.command == command && last.status == "saved" {
            return Ok(json!({"saved":true,"id":command.id}));
        }
        if last.status == "pending" && last.command != command {
            return Err(ApiError::conflict("有结果未确认的修正，请先继续核对该操作"));
        }
        if last.command == command && last.status == "rejected" {
            return Err(ApiError::invalid(
                "此调整已被宿主拒绝，请修改输入后创建新调整",
            ));
        }
    }
    if command.tokens.get() < 0 || command.requests.get() < 0 {
        let mut cursor = None;
        let mut already_written = false;
        loop {
            let page = host
                .service::<ListAggregates>(ListUsageAggregates {
                    filter: filter(command.at_ms, command.at_ms + 1),
                    cursor,
                    limit: Some(200),
                })
                .await?;
            already_written |= page.items.iter().any(|item| {
                item.identity.key == format!("kazeliu.cpr-data-toolkit:manual:{}", command.id)
            });
            match page.next_cursor {
                Some(next) if !already_written => cursor = Some(next),
                _ => break,
            }
        }
        if !already_written {
            let bucket = command.at_ms.div_euclid(900000) * 900000;
            let start = DateTime::from_timestamp_millis(bucket)
                .ok_or_else(|| ApiError::invalid("归属时间无效"))?
                .to_rfc3339();
            let end = DateTime::from_timestamp_millis(bucket + 900000)
                .ok_or_else(|| ApiError::invalid("归属时间无效"))?
                .to_rfc3339();
            let query = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("startTime", &start)
                .append_pair("endTime", &end)
                .finish();
            let current = read_summary(host, &query).await?;
            if i128::from(current.0) + i128::from(command.tokens.get()) < 0
                || i128::from(current.1) + i128::from(command.requests.get()) < 0
            {
                return Err(ApiError::invalid(
                    "减量超过首次提交时刻所属 15 分钟桶的已有量，请减少减量",
                ));
            }
        }
    }
    let mut record = AdjustmentState {
        command,
        status: "pending".into(),
    };
    if last.is_none_or(|last| last.status != "pending") {
        version = Some(state::put(host, "usage_adjustment", "last", version, &record).await?);
    }
    let command = &record.command;
    let aggregate = UsageAggregate {
        identity: UsageAggregateKey {
            key: format!("kazeliu.cpr-data-toolkit:manual:{}", command.id),
            bucket_start_ms: command.at_ms,
            dimensions: UsageDimensions::default(),
        },
        counts: BTreeMap::from([
            (UsageMetric::TotalTokens, command.tokens),
            (UsageMetric::RequestCount, command.requests),
        ]),
        costs: BTreeMap::new(),
    };
    let result = host
        .service::<WriteAggregates>(WriteUsageAggregates {
            changes: vec![UsageAggregateChange::Put { aggregate }],
        })
        .await;
    if let Err(error) = result {
        if matches!(
            error.kind.as_str(),
            "invalid_input" | "not_found" | "conflict" | "capacity_exceeded"
        ) {
            record.status = "rejected".into();
            state::put(host, "usage_adjustment", "last", version, &record).await?;
        }
        return Err(error.into());
    }
    record.status = "saved".into();
    state::put(host, "usage_adjustment", "last", version, &record).await?;
    Ok(json!({"saved":true,"id":command.id}))
}

pub async fn read_summary(host: &HostClient, query: &str) -> Result<(u64, u64)> {
    let mut response = host
        .dispatch_http(HttpRequest {
            settings: Value::Null,
            method: "GET".into(),
            uri: format!("/api/admin/usage/records/summary?{query}"),
            version: Version::Http11,
            headers: vec![],
            timeout_ms: Some(30000),
            body: HttpBody::empty(),
        })
        .await?;
    if response.status != 200 {
        response.body.close().await?;
        return Err(ApiError::new(502, "summary", "当前统计不可用"));
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body.read().await? {
        if let HttpFrame::Data(data) = frame {
            if bytes.len() + data.len() > 256 * 1024 {
                response.body.close().await?;
                return Err(ApiError::new(502, "summary", "统计响应超过容量"));
            }
            bytes.extend(data);
        }
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    let integer = |path| {
        value
            .pointer(path)
            .and_then(Value::as_u64)
            .ok_or_else(|| ApiError::new(502, "summary", "统计原值缺失或格式不正确"))
    };
    Ok((
        integer("/data/logicalRequests/totalTokens")?,
        integer("/data/logicalRequests/requestCount")?,
    ))
}
