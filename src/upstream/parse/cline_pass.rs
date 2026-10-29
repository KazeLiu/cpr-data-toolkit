//! Cline 官方套餐百分比；不推导金额或日历月秒数

use super::{QuotaWindow, invalid_response};
use crate::upstream::types::QuotaWindowRole;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashSet;

pub(super) fn parse(value: &Value) -> Result<Vec<QuotaWindow>, &'static str> {
    if value["success"] != true {
        return Err(invalid_response());
    }
    let limits = value
        .pointer("/data/limits")
        .and_then(Value::as_array)
        .ok_or_else(invalid_response)?;
    if limits.is_empty() || limits.len() > 32 {
        return Err(invalid_response());
    }
    let mut seen = HashSet::new();
    limits
        .iter()
        .map(|limit| {
            let kind = limit["type"]
                .as_str()
                .filter(|kind| {
                    !kind.is_empty() && kind.len() <= 128 && !kind.chars().any(char::is_control)
                })
                .ok_or_else(invalid_response)?;
            if !seen.insert(kind) {
                return Err(invalid_response());
            }
            let (label, role, seconds) = match kind {
                "five_hour" => (
                    "5 小时额度",
                    Some(QuotaWindowRole::Primary),
                    Some(5 * 60 * 60),
                ),
                "weekly" => (
                    "周额度",
                    Some(QuotaWindowRole::Secondary),
                    Some(7 * 24 * 60 * 60),
                ),
                "monthly" => ("月额度", Some(QuotaWindowRole::Monthly), None),
                _ => (kind, None, None),
            };
            let percent = match &limit["percentUsed"] {
                Value::Null => None,
                value => Some(
                    value
                        .as_f64()
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .ok_or_else(invalid_response)?,
                ),
            };
            let reset_at = match &limit["resetsAt"] {
                Value::Null => None,
                Value::String(text) => Some(
                    DateTime::parse_from_rfc3339(text)
                        .map_err(|_| invalid_response())?
                        .with_timezone(&Utc),
                ),
                _ => return Err(invalid_response()),
            };
            Ok(QuotaWindow {
                key: format!("cline_pass:{kind}"),
                group: "cline_pass".into(),
                label: label.into(),
                role,
                window_seconds: seconds,
                used_percent: percent.map(|percent| percent.min(100.0)),
                reset_at,
                limit_reached: percent.is_some_and(|percent| percent >= 100.0),
                limit_id: None,
                limit_name: None,
            })
        })
        .collect()
}
