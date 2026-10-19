//! CCS 来源时区与完整来源筛选，不改变来源记录身份

use crate::{
    api::{ApiError, Result},
    sources::{self, ParsedSource, UsagePreview, UsageRow},
};
use chrono::{NaiveDate, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportOptions {
    pub source_id: String,
    pub source_name: String,
    pub timezone: String,
    pub app: Option<String>,
    pub model: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

pub struct Selection {
    pub timezone: Tz,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    app: Option<String>,
    model: String,
}

impl Selection {
    pub fn new(options: &ImportOptions) -> Result<Self> {
        let timezone: Tz = options.timezone.parse().map_err(|_| {
            ApiError::invalid("请填写有效的 CCS 来源 IANA 时区，例如 Asia/Shanghai")
        })?;
        if options
            .app
            .as_ref()
            .is_some_and(|value| value.is_empty() || value.len() > 64)
            || options.model.len() > 128
        {
            return Err(ApiError::invalid("应用或模型筛选无效"));
        }
        let parse = |value: &Option<String>| -> Result<Option<i64>> {
            value
                .as_ref()
                .map(|value| {
                    let time = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
                        .map_err(|_| ApiError::invalid("时间范围须使用来源时区的日期和分钟"))?;
                    timezone
                        .from_local_datetime(&time)
                        .single()
                        .map(|value| value.timestamp_millis())
                        .ok_or_else(|| {
                            ApiError::invalid("所选时间在来源时区中不存在或有歧义，请调整时间")
                        })
                })
                .transpose()
        };
        let start_ms = parse(&options.start_time)?;
        let end_ms = parse(&options.end_time)?;
        if matches!((start_ms, end_ms), (Some(start), Some(end)) if start >= end) {
            return Err(ApiError::invalid("结束时间须晚于开始时间"));
        }
        Ok(Self {
            timezone,
            start_ms,
            end_ms,
            app: options.app.clone(),
            model: options.model.to_lowercase(),
        })
    }

    pub fn day(&self, date: &str) -> Result<(i64, i64)> {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| ApiError::invalid("来源日聚合日期无效"))?;
        let next = date
            .succ_opt()
            .ok_or_else(|| ApiError::invalid("来源日期超出范围"))?;
        let midnight = |date: NaiveDate| {
            self.timezone
                .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                .single()
                .map(|value| value.timestamp_millis())
                .ok_or_else(|| ApiError::invalid("来源日期午夜不存在或有歧义，无法确定日聚合边界"))
        };
        Ok((midnight(date)?, midnight(next)?))
    }

    pub fn contains(&self, start: i64, end: Option<i64>) -> bool {
        self.start_ms.is_none_or(|bound| start >= bound)
            && self
                .end_ms
                .is_none_or(|bound| end.map_or(start < bound, |end| end <= bound))
    }

    pub fn dimensions(&self, app: &str, model: &str) -> bool {
        self.app.as_ref().is_none_or(|value| value == app)
            && model.to_lowercase().contains(&self.model)
    }

    pub fn select(&self, source: &ParsedSource) -> Result<UsagePreview> {
        let mut partial_days = 0;
        let mut rows = Vec::new();
        for row in &source.usage.rows {
            if !self.dimensions(&row.app, &row.model) {
                continue;
            }
            let selected = if let Some(date) = &row.date {
                let (start, end) = self.day(date)?;
                let included = self.contains(start, Some(end));
                if !included
                    && self.start_ms.is_none_or(|value| end > value)
                    && self.end_ms.is_none_or(|value| start < value)
                {
                    partial_days += 1;
                }
                included
            } else {
                let time = request_ms(row)?;
                self.contains(time, None)
            };
            if selected {
                rows.push(row.clone());
            }
        }
        let mut preview = sources::summarize(rows)?;
        if partial_days > 0 {
            preview.warnings.push(format!(
                "排除 {partial_days} 行日聚合：时间范围未完整覆盖来源日期，日用量不能拆入部分时段"
            ));
        }
        Ok(preview)
    }
}

pub fn request_ms(row: &UsageRow) -> Result<i64> {
    row.time
        .and_then(|value| value.checked_mul(1000))
        .ok_or_else(|| ApiError::invalid("来源请求时间缺失或超出范围"))
}
