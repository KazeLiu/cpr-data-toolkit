//! 通过公开 SDK 在相关开始时段内核对原生请求，双方唯一对应才排除贡献

use super::{filter, selection::request_ms};
use crate::{
    api::{ApiError, Result},
    sources::UsageRow,
};
use gateway_plugin_sdk::{call::services::usage::*, client::HostClient};
use serde::Serialize;
use std::collections::HashMap;

type Signature = (String, i64, i64);
const WINDOW: i64 = 30000;
const MAX_PAGES: usize = 200;
const MAX_EDGES: usize = 100000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub started_at_ms: i64,
    pub completed_at_ms: i64,
    pub delta_ms: i64,
    pub input: String,
    pub output: String,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub source_id: String,
    pub app: String,
    pub provider: String,
    pub model: String,
    pub date: Option<String>,
    pub source_time_ms: Option<i64>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub cache_read: Option<String>,
    pub cache_write: Option<String>,
    pub excluded: bool,
    pub reason: String,
    pub candidate_count: usize,
    pub candidates: Vec<Candidate>,
}
struct Native {
    id: String,
    start: i64,
    end: i64,
    input: i64,
    output: i64,
    read: Option<i64>,
    write: Option<i64>,
}
fn model(value: &str) -> Option<String> {
    let value = value.trim().to_lowercase();
    (!value.is_empty() && value != "unknown").then_some(value)
}
fn signature(row: &UsageRow) -> Option<Signature> {
    let input = inclusive_input(row)?;
    let output = row.tokens[1]?;
    (input > 0 || output > 0).then_some((model(&row.model)?, input, output))
}
fn inclusive_input(row: &UsageRow) -> Option<i64> {
    row.tokens[0]?
        .checked_add(row.tokens[2]?)?
        .checked_add(row.tokens[3]?)
}
fn cache_matches(row: &UsageRow, native: &Native) -> bool {
    !matches!((row.tokens[2], native.read), (Some(a), Some(b)) if a != b)
        && !matches!((row.tokens[3], native.write), (Some(a), Some(b)) if a != b)
}

pub async fn inspect(
    host: &HostClient,
    rows: &[&UsageRow],
    source_rows: &[UsageRow],
    start_ms: Option<i64>,
    end_ms: Option<i64>,
) -> Result<Vec<Evidence>> {
    let mut reports = Vec::new();
    let mut eligible = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        let key = signature(row);
        let reason = if row.date.is_some() {
            "日聚合没有逐请求明细，不参与判重"
        } else if row.data_source.as_deref().unwrap_or("proxy") != "proxy" {
            "会话日志时间不能可靠视为代理完成时间"
        } else if row.requests != 1 || row.successes != 1 {
            "失败或未完成记录不自动判重"
        } else if key.is_none() {
            "模型或 Token 缺失、输入输出全零或缓存口径无法统一"
        } else {
            eligible.push((index, key.unwrap(), request_ms(row)?));
            "对应开始时段内未找到可确认的原生请求，保留导入"
        };
        reports.push(Evidence {
            source_id: row
                .request_id
                .clone()
                .unwrap_or_else(|| row.identity.clone()),
            app: row.app.clone(),
            provider: row.provider.clone(),
            model: row.model.clone(),
            date: row.date.clone(),
            source_time_ms: row.time.and_then(|value| value.checked_mul(1000)),
            input: inclusive_input(row).map(|input| input.to_string()),
            output: row.tokens[1].map(|value| value.to_string()),
            cache_read: row.tokens[2].map(|value| value.to_string()),
            cache_write: row.tokens[3].map(|value| value.to_string()),
            excluded: false,
            reason: reason.into(),
            candidate_count: 0,
            candidates: Vec::new(),
        });
    }
    if eligible.is_empty() {
        return Ok(reports);
    }
    // SDK 按开始时间过滤；只查询来源记录附近的开始时段，不扫描更早历史或假定长请求不存在
    let first = *eligible.iter().map(|(_, _, time)| time).min().unwrap();
    let last = *eligible.iter().map(|(_, _, time)| time).max().unwrap();
    let start = start_ms.unwrap_or(first).saturating_sub(WINDOW);
    let end = end_ms
        .unwrap_or(last.saturating_add(1))
        .saturating_add(WINDOW);
    let signatures: std::collections::HashSet<_> =
        eligible.iter().map(|(_, key, _)| key.clone()).collect();
    let mut groups: HashMap<Signature, Vec<usize>> = HashMap::new();
    let mut native = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    for page_number in 0..MAX_PAGES {
        let page = host
            .service::<ListRecords>(ListUsageRecords {
                filter: filter(start, end),
                cursor,
                limit: Some(50),
            })
            .await?;
        for record in page.items {
            let data = record.data;
            if !matches!(data.outcome, UsageOutcome::Succeeded) {
                continue;
            }
            let Some(completed) = data.completed_at_ms else {
                continue;
            };
            let (Some(input), Some(output)) = (data.tokens.input_tokens, data.tokens.output_tokens)
            else {
                continue;
            };
            let (Ok(input), Ok(output)) = (i64::try_from(input.0), i64::try_from(output.0)) else {
                continue;
            };
            let Some(name) = data
                .model
                .as_deref()
                .or(data.requested_model.as_deref())
                .and_then(model)
            else {
                continue;
            };
            let key = (name, input, output);
            if !signatures.contains(&key) || !seen.insert(record.id.clone()) {
                continue;
            }
            groups.entry(key).or_default().push(native.len());
            native.push(Native {
                id: record.id,
                start: data.started_at_ms,
                end: completed,
                input,
                output,
                read: data
                    .tokens
                    .cached_input_tokens
                    .and_then(|value| i64::try_from(value.0).ok()),
                write: data
                    .tokens
                    .cache_write_input_tokens
                    .and_then(|value| i64::try_from(value.0).ok()),
            });
        }
        match page.next_cursor {
            None => break,
            Some(next) if page_number + 1 < MAX_PAGES => cursor = Some(next),
            Some(_) => {
                return Err(ApiError::invalid(
                    "原生明细核对超过 10000 条，请缩小时间范围后重新预览",
                ));
            }
        }
    }
    for indices in groups.values_mut() {
        indices.sort_by_key(|index| native[*index].end);
    }
    let mut edges = vec![Vec::new(); rows.len()];
    let mut degrees = vec![0usize; native.len()];
    let mut edge_count = 0;
    for (index, key, time) in eligible {
        if let Some(indices) = groups.get(&key) {
            let lower =
                indices.partition_point(|index| native[*index].end < time.saturating_sub(WINDOW));
            let upper =
                indices.partition_point(|index| native[*index].end <= time.saturating_add(WINDOW));
            for &candidate in &indices[lower..upper] {
                let value = &native[candidate];
                let row = &rows[index];
                if !cache_matches(row, value) {
                    continue;
                }
                edge_count += 1;
                if edge_count > MAX_EDGES {
                    return Err(ApiError::invalid(
                        "相似请求候选过多，请缩小时间范围后重新预览",
                    ));
                }
                edges[index].push(candidate);
                degrees[candidate] += 1;
            }
        }
    }
    // 未选中记录不参与导入，但不能因切换筛选范围而让同一原生请求被多次认领
    let selected: std::collections::HashSet<_> =
        rows.iter().map(|row| row.identity.as_str()).collect();
    for row in source_rows {
        if selected.contains(row.identity.as_str())
            || row.internal_duplicate
            || row.date.is_some()
            || row.data_source.as_deref().unwrap_or("proxy") != "proxy"
            || row.requests != 1
            || row.successes != 1
        {
            continue;
        }
        let (Some(key), Some(time)) = (
            signature(row),
            row.time.and_then(|value| value.checked_mul(1000)),
        ) else {
            continue;
        };
        if let Some(indices) = groups.get(&key) {
            let lower =
                indices.partition_point(|index| native[*index].end < time.saturating_sub(WINDOW));
            let upper =
                indices.partition_point(|index| native[*index].end <= time.saturating_add(WINDOW));
            for &index in &indices[lower..upper] {
                let candidate = &native[index];
                if !cache_matches(row, candidate) {
                    continue;
                }
                edge_count += 1;
                if edge_count > MAX_EDGES {
                    return Err(ApiError::invalid(
                        "相似请求候选过多，请缩小来源范围后重新预览",
                    ));
                }
                degrees[index] += 1;
            }
        }
    }
    for (index, candidates) in edges.into_iter().enumerate() {
        if candidates.is_empty() {
            continue;
        }
        let report = &mut reports[index];
        report.candidate_count = candidates.len();
        report.excluded = candidates.len() == 1 && degrees[candidates[0]] == 1;
        report.reason = if report.excluded {
            "时间、模型及 Token 唯一对应，排除来源贡献"
        } else {
            "存在多对一或多候选，无法唯一对应，保留导入"
        }
        .into();
        for candidate in candidates.into_iter().take(20) {
            let value = &native[candidate];
            report.candidates.push(Candidate {
                id: value.id.clone(),
                started_at_ms: value.start,
                completed_at_ms: value.end,
                delta_ms: report.source_time_ms.unwrap() - value.end,
                input: value.input.to_string(),
                output: value.output.to_string(),
                cache_read: value.read.map(|value| value.to_string()),
                cache_write: value.write.map(|value| value.to_string()),
            });
        }
    }
    Ok(reports)
}
