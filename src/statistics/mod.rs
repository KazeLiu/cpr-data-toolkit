//! 按维度推进聚合报表，每次 SDK 快照在当前管理父调用内读完

mod report;
mod source;

use crate::{
    api::{ApiError, Result},
    upstream::now,
    usage,
};
use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use gateway_plugin_sdk::{
    call::services::usage::{UsageGrouping, UsageRecordFilter},
    client::HostClient,
};
use report::{Dimension, Query, Report, Row};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::{Duration as StdDuration, Instant},
};
use tokio::sync::{Mutex, Notify};

struct JobHandle {
    data: Mutex<Job>,
    cancel: Notify,
}
type Jobs = Mutex<BTreeMap<String, Arc<JobHandle>>>;
static JOBS: OnceLock<Jobs> = OnceLock::new();
const TTL: StdDuration = StdDuration::from_secs(15 * 60);
const PAGE_SIZE: usize = 50;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Keys,
    Directory,
    Statistics,
    Groups,
    Complete,
}

struct Job {
    id: String,
    timezone: String,
    filter: UsageRecordFilter,
    phase: Phase,
    cursor: Option<String>,
    group_page: usize,
    step: usize,
    scanned: usize,
    touched: Instant,
    blocked: bool,
    report: Report,
}

impl Job {
    fn view(&self) -> Value {
        let zone: Tz = self.timezone.parse().unwrap();
        let display = |milliseconds| {
            DateTime::from_timestamp_millis(milliseconds)
                .unwrap()
                .with_timezone(&zone)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        };
        json!({"taskId":self.id,"step":self.step,"phase":match self.phase {Phase::Keys=>"keys",Phase::Directory=>"directory",Phase::Statistics=>"statistics",Phase::Groups=>"groups",Phase::Complete=>"complete"},
            "scanned":self.scanned,"keys":self.report.keys.len(),"timezone":self.timezone,
            "startMs":self.filter.start_ms,"endMs":self.filter.end_ms,"startDisplay":display(self.filter.start_ms),"endDisplay":display(self.filter.end_ms),"blocked":self.blocked})
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Start {
    timezone: String,
    start_time: String,
    end_time: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Task {
    task_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Step {
    task_id: String,
    step: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    timezone: String,
}

fn timezone(value: &str) -> Result<Tz> {
    value
        .parse()
        .map_err(|_| ApiError::invalid("统计时区无效，请使用 IANA 时区名称"))
}

fn local(value: &str, zone: Tz) -> Result<DateTime<Utc>> {
    let date = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .map_err(|_| ApiError::invalid("日期时间格式应为 YYYY-MM-DDTHH:mm"))?;
    zone.from_local_datetime(&date)
        .single()
        .map(|time| time.with_timezone(&Utc))
        .ok_or_else(|| {
            ApiError::invalid("所选时间在该时区不存在或有夏令时歧义，请调整时间或使用 UTC")
        })
}

pub fn metadata(input: Metadata) -> Result<Value> {
    let zone = timezone(&input.timezone)?;
    let today = now().with_timezone(&zone).date_naive();
    let month = today.with_day(1).unwrap();
    let next_month = (month + Duration::days(32)).with_day(1).unwrap();
    let previous_month = (month - Duration::days(1)).with_day(1).unwrap();
    let presets = [
        ("today","今天",today,today+Duration::days(1)),
        ("yesterday","昨天",today-Duration::days(1),today),
        ("7d","近 7 天",today-Duration::days(6),today+Duration::days(1)),
        ("month","本月",month,next_month),
        ("last_month","上月",previous_month,month),
    ].into_iter().map(|(id,name,start,end)| json!({"id":id,"name":name,"range":[format!("{start}T00:00"),format!("{end}T00:00")]})).collect::<Vec<_>>();
    Ok(
        json!({"timezone":input.timezone,"presets":presets,"timezones":chrono_tz::TZ_VARIANTS.iter().map(|zone|zone.name()).collect::<Vec<_>>()}),
    )
}

pub async fn expire() {
    let mut jobs = JOBS.get_or_init(Default::default).lock().await;
    jobs.retain(|_, job| {
        job.data
            .try_lock()
            .map_or(true, |job| job.touched.elapsed() < TTL)
    });
}

pub async fn start(input: Start) -> Result<Value> {
    let zone = timezone(&input.timezone)?;
    let start = local(&input.start_time, zone)?.timestamp_millis();
    // 固定用户时间上界，各维度另有宿主返回的快照锚点与投影代次
    let end = local(&input.end_time, zone)?
        .timestamp_millis()
        .min(now().timestamp_millis());
    if start >= end {
        return Err(ApiError::invalid("开始时间必须早于结束时间和当前时刻"));
    }
    expire().await;
    let mut jobs = JOBS.get_or_init(Default::default).lock().await;
    if jobs.len() >= 2 {
        return Err(ApiError::conflict(
            "已有两个统计任务，请先取消其中一个或等待闲置清理",
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let job = Job {
        id: id.clone(),
        timezone: input.timezone,
        filter: usage::filter(start, end),
        phase: Phase::Keys,
        cursor: None,
        group_page: 1,
        step: 0,
        scanned: 0,
        touched: Instant::now(),
        blocked: false,
        report: Report::default(),
    };
    let view = job.view();
    jobs.insert(
        id,
        Arc::new(JobHandle {
            data: Mutex::new(job),
            cancel: Notify::new(),
        }),
    );
    Ok(view)
}

async fn job(id: &str) -> Result<Arc<JobHandle>> {
    expire().await;
    JOBS.get_or_init(Default::default)
        .lock()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| {
            ApiError::new(
                404,
                "statistics_expired",
                "统计任务已取消或过期，请重新查询",
            )
        })
}

pub async fn cancel(input: Task) -> Result<Value> {
    if let Some(handle) = JOBS
        .get_or_init(Default::default)
        .lock()
        .await
        .remove(&input.task_id)
    {
        handle.cancel.notify_one();
    }
    Ok(json!({"cancelled":true}))
}

pub async fn step(host: &HostClient, input: Step) -> Result<Value> {
    let handle = job(&input.task_id).await?;
    let mut job = handle.data.lock().await;
    job.touched = Instant::now();
    if input.step > job.step {
        return Err(ApiError::conflict("统计步骤不匹配，请重新查询"));
    }
    if job.blocked {
        return Err(ApiError::invalid("统计汇总未完成，请缩小范围后重新查询"));
    }
    // 重复提交已确认的步骤只回读进度，响应丢失后继续不会重复累加同一页
    if input.step < job.step || job.phase == Phase::Complete {
        return Ok(job.view());
    }
    let filter = job.filter.clone();
    match job.phase {
        Phase::Keys => {
            let page = source::keys(host, job.cursor.as_deref()).await?;
            for key in page.items {
                job.report.keys.insert(key.id.clone(), key);
            }
            job.cursor = page.next_cursor;
            if job.cursor.is_none() {
                job.phase = Phase::Directory;
            }
        }
        Phase::Directory => {
            let page = source::groups(host, job.group_page).await?;
            for group in page.items {
                job.report.groups.insert(group.id.clone(), group);
            }
            if job.group_page >= page.page.total_pages {
                job.phase = Phase::Statistics;
            } else {
                job.group_page += 1;
            }
        }
        Phase::Statistics => {
            let data =
                source::statistics(host, filter, UsageGrouping::KeyModel, None, &handle.cancel)
                    .await?;
            job.scanned += data.len();
            job.report.models = data;
            job.phase = Phase::Groups;
        }
        Phase::Groups => {
            let data = source::statistics(
                host,
                filter,
                UsageGrouping::RoutingGroup,
                None,
                &handle.cancel,
            )
            .await?;
            job.scanned += data.len();
            job.report.routing = data;
            job.phase = Phase::Complete;
        }
        Phase::Complete => unreachable!(),
    }
    job.step += 1;
    job.touched = Instant::now();
    Ok(job.view())
}

async fn prepare_detail(
    host: &HostClient,
    job: &mut Job,
    input: &Query,
    cancel: &Notify,
) -> Result<()> {
    if let Some(group) = job.report.group_filter(input)? {
        let parent = input.parent.as_ref().unwrap();
        if !job
            .report
            .detail
            .as_ref()
            .is_some_and(|(id, _)| id == &parent.id)
        {
            let data = source::statistics(
                host,
                job.filter.clone(),
                UsageGrouping::KeyModel,
                Some(group),
                cancel,
            )
            .await?;
            job.report.detail = Some((parent.id.clone(), data));
        }
    }
    Ok(())
}

pub async fn query(host: &HostClient, input: Query) -> Result<Value> {
    let handle = job(&input.task_id).await?;
    let mut job = handle.data.lock().await;
    job.touched = Instant::now();
    if job.phase != Phase::Complete || job.blocked {
        return Err(ApiError::conflict(
            "统计尚未完成，不能作为完整结果查看或导出",
        ));
    }
    prepare_detail(host, &mut job, &input, &handle.cancel).await?;
    let (summary, rows) = job.report.rows(&input)?;
    let items = rows
        .iter()
        .skip(input.offset)
        .take(PAGE_SIZE)
        .map(Row::view)
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"summary":summary.view()?,"items":items,"total":rows.len(),"offset":input.offset,"pageSize":PAGE_SIZE,"observation":job.report.snapshot(&input)?}),
    )
}

fn csv_field(value: &str) -> String {
    let formula = value
        .trim_start()
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@'))
        && value.trim().parse::<rust_decimal::Decimal>().is_err();
    format!(
        "\"{}{}\"",
        if formula { "'" } else { "" },
        value.replace('"', "\"\"")
    )
}

pub async fn export(host: &HostClient, input: Query) -> Result<Value> {
    let handle = job(&input.task_id).await?;
    let mut job = handle.data.lock().await;
    job.touched = Instant::now();
    if job.phase != Phase::Complete || job.blocked {
        return Err(ApiError::conflict("统计尚未完成，不能导出"));
    }
    prepare_detail(host, &mut job, &input, &handle.cancel).await?;
    let (_, rows) = job.report.rows(&input)?;
    let prepared = rows
        .into_iter()
        .map(|row| {
            let totals = row.totals.view()?;
            Ok((row, totals))
        })
        .collect::<Result<Vec<_>>>()?;
    let currencies = prepared
        .iter()
        .flat_map(|(_, totals)| {
            totals["costs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cost| cost["currency"].as_str().unwrap().to_owned())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut header = [
        "统计时区",
        "开始 UTC",
        "结束 UTC（不含）",
        "名称",
        "分组身份",
        "请求数",
        "成功",
        "失败",
        "取消",
        "未完成",
        "状态未细分",
        "输入 Token",
        "输出 Token",
        "缓存读取",
        "缓存写入",
        "总 Token",
        "缓存读取占比",
        "原生未计价请求",
        "输入缺失项",
        "输出缺失项",
        "缓存读缺失项",
        "缓存写缺失项",
        "总 Token 缺失项",
        "上游回报请求数",
        "宿主估算请求数",
        "快照观察信息",
    ]
    .map(str::to_owned)
    .to_vec();
    for currency in &currencies {
        header.extend(
            ["已知费用", "原生费用", "聚合贡献"].map(|label| format!("{currency} {label}")),
        );
    }
    let mut csv = format!(
        "\u{feff}{}\r\n",
        header
            .iter()
            .map(|value| csv_field(value))
            .collect::<Vec<_>>()
            .join(",")
    );
    let start = DateTime::from_timestamp_millis(job.filter.start_ms)
        .unwrap()
        .to_rfc3339();
    let end = DateTime::from_timestamp_millis(job.filter.end_ms)
        .unwrap()
        .to_rfc3339();
    for (row, totals) in prepared {
        let text = |key: &str| totals[key].as_str().unwrap_or_default().to_owned();
        let mut fields = vec![
            job.timezone.clone(),
            start.clone(),
            end.clone(),
            row.name,
            row.id,
        ];
        fields.extend(
            [
                "requests",
                "succeeded",
                "failed",
                "cancelled",
                "incomplete",
                "unknownOutcomes",
            ]
            .map(text),
        );
        fields.extend(
            ["input", "output", "cacheRead", "cacheWrite", "tokens"].map(|key| {
                if totals[key]["known"] == 0 {
                    "未知".into()
                } else {
                    totals[key]["value"].as_str().unwrap_or_default().to_owned()
                }
            }),
        );
        fields.push(
            totals["cacheRatio"]
                .as_f64()
                .map(|v| format!("{:.2}%", v * 100.0))
                .unwrap_or_else(|| "未知".into()),
        );
        fields.push(text("unpriced"));
        fields.extend(
            ["input", "output", "cacheRead", "cacheWrite", "tokens"]
                .map(|key| totals[key]["missing"].to_string()),
        );
        fields.push(totals["reportedCount"].to_string());
        fields.push(totals["calculatedCount"].to_string());
        fields.push(job.report.snapshot(&input)?.to_string());
        for currency in &currencies {
            let cost = totals["costs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|cost| cost["currency"].as_str() == Some(currency.as_str()));
            fields.extend(["amount", "native", "contribution"].map(|key| {
                cost.and_then(|cost| cost[key].as_str())
                    .unwrap_or_default()
                    .to_owned()
            }));
        }
        csv.push_str(
            &fields
                .iter()
                .map(|value| csv_field(value))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push_str("\r\n");
        if csv.len() > 8 * 1024 * 1024 {
            return Err(ApiError::invalid(
                "CSV 超过 8 MiB，请缩小时间范围或筛选后导出",
            ));
        }
    }
    let dimension = match input.dimension {
        Dimension::Model => "model",
        Dimension::Key => "key",
        Dimension::Group => "group",
    };
    Ok(
        json!({"filename":format!("usage-{dimension}-{}.csv",now().format("%Y%m%d-%H%M%S")),"csv":csv}),
    )
}
