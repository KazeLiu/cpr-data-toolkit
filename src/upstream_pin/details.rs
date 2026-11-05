//! 仅保留有界的结构化诊断字段，不采集回答、工具参数、请求头或上游错误正文

use super::Entry;
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub request_id: String,
    pub attempt_index: u32,
}

fn filtered(value: &Value, depth: usize, budget: &mut usize, limited: &mut bool) -> Option<Value> {
    if depth > 8 || *budget == 0 {
        *limited = true;
        return None;
    }
    *budget -= 1;
    match value {
        Value::Object(object) => {
            let mut output = Map::new();
            for (key, value) in object {
                if matches!(
                    key.as_str(),
                    "gateway"
                        | "routing"
                        | "finalProvider"
                        | "resolvedProvider"
                        | "canonicalSlug"
                        | "model"
                        | "provider"
                        | "modelId"
                        | "providerId"
                        | "modelSlug"
                        | "modelAttemptCount"
                        | "providerAttemptCount"
                        | "modelAttempts"
                        | "providerAttempts"
                        | "attempts"
                        | "models"
                        | "success"
                        | "status"
                        | "statusCode"
                        | "code"
                        | "type"
                        | "param"
                        | "fallbackProviders"
                        | "fallbacksAvailable"
                        | "availableProviders"
                        | "affinity"
                        | "outcome"
                        | "scope"
                        | "geoRegion"
                        | "region"
                        | "cost"
                        | "gatewayCost"
                        | "marketCost"
                        | "inferenceCost"
                        | "currency"
                        | "prompt_tokens"
                        | "completion_tokens"
                        | "input_tokens"
                        | "output_tokens"
                        | "total_tokens"
                        | "prompt_tokens_details"
                        | "completion_tokens_details"
                        | "input_tokens_details"
                        | "output_tokens_details"
                        | "cached_tokens"
                        | "reasoning_tokens"
                        | "cache_write_tokens"
                        | "prompt_cache_hit_tokens"
                        | "prompt_cache_miss_tokens"
                        | "cache_read_input_tokens"
                        | "cache_creation_input_tokens"
                ) && let Some(value) = filtered(value, depth + 1, budget, limited)
                {
                    output.insert(key.clone(), value);
                }
            }
            Some(Value::Object(output))
        }
        Value::Array(values) => {
            *limited |= values.len() > 32;
            Some(Value::Array(
                values
                    .iter()
                    .take(32)
                    .filter_map(|value| filtered(value, depth + 1, budget, limited))
                    .collect(),
            ))
        }
        Value::String(text) if text.len() <= 512 && !text.chars().any(char::is_control) => {
            Some(value.clone())
        }
        Value::String(_) => {
            *limited = true;
            None
        }
        _ => Some(value.clone()),
    }
}

fn merge(target: &mut Map<String, Value>, update: Map<String, Value>) {
    for (key, value) in update {
        if value.is_null() {
            continue;
        }
        match value {
            Value::Object(update) => {
                if let Some(Value::Object(current)) = target.get_mut(&key) {
                    merge(current, update);
                } else {
                    target.insert(key, Value::Object(update));
                }
            }
            value => {
                target.insert(key, value);
            }
        }
    }
}

pub(super) fn observe(entry: &mut Entry, raw: &Value) {
    let value = if raw["success"] == true {
        &raw["data"]
    } else {
        raw.get("response").unwrap_or(raw)
    };
    // Responses 包装后的 model 是适配器生成的请求模型，不能冒充真实 Chat 响应模型
    if value["object"] != "response"
        && let Some(model) = value["model"]
            .as_str()
            .filter(|s| super::valid_text(s, 512))
    {
        entry.served_model = Some(model.into());
    }
    let delta = value
        .pointer("/choices/0/delta")
        .or_else(|| value.pointer("/choices/0/message"));
    let effective = delta.is_some_and(|delta| {
        ["content", "reasoning", "reasoning_content", "refusal"]
            .iter()
            .any(|key| delta[*key].as_str().is_some_and(|s| !s.is_empty()))
            || delta["tool_calls"]
                .as_array()
                .is_some_and(|calls| !calls.is_empty())
    }) || matches!(
        raw["type"].as_str(),
        Some(
            "response.output_text.delta"
                | "response.reasoning_summary_text.delta"
                | "response.function_call_arguments.delta"
        )
    );
    if effective && entry.ttft_ms.is_none() {
        entry.ttft_ms = entry
            .started
            .map(|start| start.elapsed().as_millis() as u64);
    }
    let mut update = Map::new();
    for (key, item) in [
        ("usage", value.get("usage")),
        ("error", value.get("error")),
        (
            "provider_metadata",
            value
                .get("provider_metadata")
                .filter(|v| !v.is_null())
                .or_else(|| value.get("providerMetadata")),
        ),
    ] {
        if let Some(item) = item
            .filter(|v| v.is_object())
            .and_then(|v| filtered(v, 0, &mut 512, &mut entry.detail_limited))
        {
            update.insert(key.into(), item);
        }
    }
    if let Some(delta) = delta {
        if let Some(item) = delta
            .get("provider_metadata")
            .filter(|v| !v.is_null())
            .or_else(|| delta.get("providerMetadata"))
            .and_then(|v| filtered(v, 0, &mut 512, &mut entry.detail_limited))
        {
            let mut combined = Map::new();
            combined.insert("provider_metadata".into(), item);
            merge(&mut update, combined);
        }
    }
    if let Some(reason) = value
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        .filter(|s| super::valid_text(s, 128))
    {
        update.insert("finish_reason".into(), Value::String(reason.into()));
    }
    if let Some(tier) = value["service_tier"]
        .as_str()
        .filter(|s| super::valid_text(s, 128))
    {
        update.insert("service_tier".into(), Value::String(tier.into()));
    }
    if !update.is_empty() {
        if update
            .iter()
            .all(|(key, value)| entry.response.get(key) == Some(value))
        {
            return;
        }
        let mut candidate = entry.response.clone();
        merge(&mut candidate, update);
        if serde_json::to_vec(&candidate).is_ok_and(|bytes| bytes.len() <= 32 * 1024) {
            entry.response = candidate;
        } else {
            entry.detail_limited = true;
        }
    }
}
