//! Cline 路由条件的发送前注入；日志只保存规则和明确回报的最终提供方

use crate::{
    api::{ApiError, Result},
    state, upstream,
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::middleware::{MiddlewareBodyFraming, outbound::Target},
    client::{HostClient, MiddlewareResponse, OutboundCall, OutboundRequest, RequestCall},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

const NAMESPACE: &str = "upstream_pin";
const CAPACITY: usize = 200;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    account_id: String,
    model: String,
    enabled: bool,
    providers: Vec<String>,
    has: Vec<String>,
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
    account_id: String,
    model: String,
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}

fn normalize_slugs(values: &mut [String]) -> Result<()> {
    if values.len() > 8 {
        return Err(ApiError::invalid("提供方和能力各最多填写 8 项"));
    }
    let mut seen = HashSet::new();
    for value in values {
        *value = value.trim().to_ascii_lowercase();
        if !valid_text(value, 128)
            || value.chars().any(char::is_whitespace)
            || !seen.insert(value.clone())
        {
            return Err(ApiError::invalid("提供方和能力须为不重复、无空白的短名称"));
        }
    }
    Ok(())
}

async fn load(host: &HostClient) -> std::result::Result<(Option<u64>, Configuration), PluginFault> {
    let (version, value) = state::get(host, NAMESPACE, "rules").await?;
    Ok((version, value.unwrap_or_default()))
}

pub async fn get(host: &HostClient) -> Result<Value> {
    let (version, config) = load(host).await?;
    Ok(json!({"version":version,"rules":config.rules}))
}

fn require_version(current: Option<u64>, expected: Option<u64>) -> Result<()> {
    if current != expected {
        return Err(ApiError::conflict("钉选规则已更新，请重新读取后修改"));
    }
    Ok(())
}

pub async fn save(host: &HostClient, mut input: Save) -> Result<Value> {
    if !valid_text(&input.rule.account_id, 256) || !valid_text(&input.rule.model, 512) {
        return Err(ApiError::invalid("请选择账号并填写完整上游模型 ID"));
    }
    normalize_slugs(&mut input.rule.providers)?;
    normalize_slugs(&mut input.rule.has)?;
    if input.rule.providers.is_empty() {
        return Err(ApiError::invalid("至少填写一个允许的提供方"));
    }
    let account = upstream::runtime(host, &input.rule.account_id).await?;
    if account.provider_id != "openai" || account.authentication_kind != "api_key" {
        return Err(ApiError::invalid("钉上游仅支持 OpenAI API Key 账号"));
    }
    let (version, mut config) = load(host).await?;
    require_version(version, input.expected_version)?;
    if let Some(rule) = config
        .rules
        .iter_mut()
        .find(|rule| rule.account_id == input.rule.account_id && rule.model == input.rule.model)
    {
        *rule = input.rule;
    } else {
        if config.rules.len() >= 100 {
            return Err(ApiError::invalid("最多保存 100 条钉选规则"));
        }
        config.rules.push(input.rule);
    }
    if serde_json::to_vec(&config)?.len() > 240 * 1024 {
        return Err(ApiError::invalid("钉选规则超过保存容量"));
    }
    let version = state::put(host, NAMESPACE, "rules", version, &config).await?;
    Ok(json!({"version":version,"rules":config.rules}))
}

pub async fn delete(host: &HostClient, input: Delete) -> Result<Value> {
    let (version, mut config) = load(host).await?;
    require_version(version, input.expected_version)?;
    config
        .rules
        .retain(|rule| rule.account_id != input.account_id || rule.model != input.model);
    let version = state::put(host, NAMESPACE, "rules", version, &config).await?;
    Ok(json!({"version":version,"rules":config.rules}))
}

fn invalid_body() -> PluginFault {
    PluginFault::new(
        ErrorCode::InvalidInput,
        "钉上游需要未压缩的 Responses JSON 请求",
    )
}

// 只接管 Cline 的路由命名空间，其他 providerOptions、消息和工具保留原值
fn apply(body: &mut Map<String, Value>, rule: &Rule) -> std::result::Result<(), PluginFault> {
    let options = body.entry("providerOptions").or_insert_with(|| json!({}));
    if options.is_null() {
        *options = json!({});
    }
    let options = options.as_object_mut().ok_or_else(invalid_body)?;
    let mut gateway = Map::new();
    gateway.insert("only".into(), json!(rule.providers));
    gateway.insert("order".into(), json!(rule.providers));
    if !rule.has.is_empty() {
        gateway.insert("has".into(), json!(rule.has));
    }
    options.insert("gateway".into(), Value::Object(gateway));
    body.insert(
        "provider".into(),
        json!({"only":rule.providers,"order":rule.providers}),
    );
    Ok(())
}

pub async fn outbound(
    logs: &Logs,
    call: OutboundCall,
) -> std::result::Result<OutboundRequest, PluginFault> {
    let mut request = call.request;
    if call.metadata.provider != "openai" {
        return call.next.run(request).await;
    }
    let Target::Http { method, uri } = &request.head.target else {
        return call.next.run(request).await;
    };
    let is_responses = method == "POST"
        && url::Url::parse(uri).is_ok_and(|url| url.path().ends_with("/responses"));
    if !is_responses {
        return call.next.run(request).await;
    }
    let (_, config) = load(&call.host).await?;
    let rules: Vec<_> = config
        .rules
        .iter()
        .filter(|rule| rule.enabled && rule.account_id == call.metadata.account_id)
        .collect();
    if rules.is_empty() {
        return call.next.run(request).await;
    }
    let targets_model = rules
        .iter()
        .any(|rule| call.metadata.model.as_deref() == Some(rule.model.as_str()));
    if request.head.headers.iter().any(|header| {
        header.name.eq_ignore_ascii_case("content-encoding")
            && !header.value.eq_ignore_ascii_case(b"identity")
    }) {
        return if targets_model {
            Err(invalid_body())
        } else {
            call.next.run(request).await
        };
    }
    let mut body: Value = match serde_json::from_slice(&request.body) {
        Ok(body) => body,
        Err(_) if !targets_model => return call.next.run(request).await,
        Err(_) => return Err(invalid_body()),
    };
    let Some(rule) = rules
        .into_iter()
        .find(|rule| body.get("model").and_then(Value::as_str) == Some(rule.model.as_str()))
    else {
        return call.next.run(request).await;
    };
    apply(body.as_object_mut().ok_or_else(invalid_body)?, rule)?;
    request.body = serde_json::to_vec(&body).map_err(|_| invalid_body())?;
    // 正文长度变化后由宿主 HTTP 客户端重新计算，不保留旧 Content-Length
    request
        .head
        .headers
        .retain(|header| !header.name.eq_ignore_ascii_case("content-length"));
    let key = (call.metadata.request_id, call.metadata.attempt_index);
    logs.record(&key, rule);
    let result = call.next.run(request).await;
    if result.is_err() {
        logs.failed(&key);
    }
    result
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    request_id: String,
    attempt_index: u32,
    time_ms: u64,
    account_id: String,
    model: String,
    providers: Vec<String>,
    has: Vec<String>,
    final_provider: Option<String>,
    failed: bool,
}

#[derive(Default)]
pub struct Logs {
    entries: Mutex<VecDeque<Entry>>,
}

impl Logs {
    fn record(&self, key: &(String, u32), rule: &Rule) {
        let mut entries = self.entries.lock().expect("pin log lock");
        entries.retain(|entry| entry.request_id != key.0 || entry.attempt_index != key.1);
        entries.push_front(Entry {
            request_id: key.0.clone(),
            attempt_index: key.1,
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_millis() as u64,
            account_id: rule.account_id.clone(),
            model: rule.model.clone(),
            providers: rule.providers.clone(),
            has: rule.has.clone(),
            final_provider: None,
            failed: false,
        });
        entries.truncate(CAPACITY);
    }
    fn contains(&self, key: &(String, u32)) -> bool {
        self.entries
            .lock()
            .expect("pin log lock")
            .iter()
            .any(|entry| entry.request_id == key.0 && entry.attempt_index == key.1)
    }
    fn failed(&self, key: &(String, u32)) {
        if let Some(entry) = self
            .entries
            .lock()
            .expect("pin log lock")
            .iter_mut()
            .find(|entry| entry.request_id == key.0 && entry.attempt_index == key.1)
        {
            entry.failed = true;
        }
    }
    fn observe(&self, key: &(String, u32), value: &Value) {
        let provider =
            reported_provider(value).or_else(|| value.get("data").and_then(reported_provider));
        let Some(provider) = provider.filter(|provider| {
            valid_text(provider, 128) && !provider.chars().any(char::is_whitespace)
        }) else {
            return;
        };
        if let Some(entry) = self
            .entries
            .lock()
            .expect("pin log lock")
            .iter_mut()
            .find(|entry| entry.request_id == key.0 && entry.attempt_index == key.1)
        {
            entry.final_provider = Some(provider.to_owned());
        }
    }
    pub fn page(&self, offset: usize) -> Result<Value> {
        if offset % 25 != 0 {
            return Err(ApiError::invalid("日志分页位置无效"));
        }
        let entries = self.entries.lock().expect("pin log lock");
        Ok(
            json!({"entries":entries.iter().skip(offset).take(25).collect::<Vec<_>>(),"total":entries.len(),"capacity":CAPACITY,"pageSize":25}),
        )
    }
    pub fn clear(&self) {
        self.entries.lock().expect("pin log lock").clear();
    }
}

fn reported_provider(value: &Value) -> Option<&str> {
    fn at(value: &Value) -> Option<&str> {
        value
            .pointer("/provider_metadata/gateway/routing/finalProvider")
            .or_else(|| value.pointer("/providerMetadata/gateway/routing/finalProvider"))?
            .as_str()
    }
    at(value).or_else(|| value.get("response").and_then(at))
}

pub async fn observe(
    logs: Arc<Logs>,
    call: RequestCall,
) -> std::result::Result<MiddlewareResponse, PluginFault> {
    let key = (
        call.request.head.request_id.clone(),
        call.request.head.attempt_index.expect("attempt mount"),
    );
    let result = call.next.run(call.request).await;
    let mut response = match result {
        Ok(response) => response,
        Err(error) => {
            logs.failed(&key);
            return Err(error);
        }
    };
    if !logs.contains(&key) {
        return Ok(response);
    }
    if response.status >= 400 {
        logs.failed(&key);
    }
    let framing = response.body.framing();
    if !matches!(
        framing,
        Some(MiddlewareBodyFraming::JsonDocument | MiddlewareBodyFraming::SseEvent)
    ) {
        return Ok(response);
    }
    response.body = response.body.inspect_frames(move |frame| {
        let value = match framing {
            Some(MiddlewareBodyFraming::SseEvent) => {
                std::str::from_utf8(&frame.payload).ok().and_then(|text| {
                    let data = text
                        .lines()
                        .filter_map(|line| {
                            line.strip_prefix("data:")
                                .map(|line| line.strip_prefix(' ').unwrap_or(line))
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    serde_json::from_str::<Value>(&data).ok()
                })
            }
            _ => serde_json::from_slice::<Value>(&frame.payload).ok(),
        };
        if let Some(value) = value {
            if !value["error"].is_null() || value["type"] == "response.failed" {
                logs.failed(&key);
            }
            logs.observe(&key, &value);
        }
    })?;
    Ok(response)
}
