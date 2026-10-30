//! 仅接管明确选择的 OpenAI API Key 账号；网络认证和调度仍归宿主

mod request;
mod response;

use crate::{
    api::{ApiError, Result},
    state, upstream, upstream_pin,
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        model::{CanonicalEvent, ExecutionEvent, Usage, facts::ResponseObservation},
        upstream_adapter::*,
    },
    client::{
        Empty, HostClient, HostHttpBody, PullResponseFuture, PullResponseStream, ResponseStream,
        TypedCall, TypedReply,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};

const NAMESPACE: &str = "cline_adapter";
const ADAPTER: &str = "cline-chat";

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configuration {
    account_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceConfiguration {
    #[serde(default)]
    pub cline_account_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    expected_version: Option<u64>,
    account_ids: Vec<String>,
}

pub struct Adapter {
    active: Vec<String>,
    maximum_stream_chunk_bytes: usize,
}

async fn load(host: &HostClient) -> std::result::Result<(Option<u64>, Configuration), PluginFault> {
    let (version, value) = state::get(host, NAMESPACE, "accounts").await?;
    Ok((version, value.unwrap_or_default()))
}

impl Adapter {
    pub fn new(maximum_stream_chunk_bytes: usize, mut account_ids: Vec<String>) -> Self {
        account_ids.sort();
        Self {
            active: account_ids,
            maximum_stream_chunk_bytes,
        }
    }
    fn view(&self, version: Option<u64>, mut config: Configuration) -> Value {
        if version.is_none() {
            config.account_ids.clone_from(&self.active);
        }
        json!({"version":version,"accountIds":config.account_ids,"activeAccountIds":self.active,
            "pendingConfiguration":config.account_ids!=self.active})
    }
    pub async fn get(&self, host: &HostClient) -> Result<Value> {
        let (version, config) = load(host).await?;
        Ok(self.view(version, config))
    }
    pub async fn save(&self, host: &HostClient, mut input: Save) -> Result<Value> {
        if input.account_ids.len() > 100 {
            return Err(ApiError::invalid("最多选择 100 个 Cline 转换账号"));
        }
        let mut seen = HashSet::new();
        let (_, old) = load(host).await?;
        for id in &input.account_ids {
            if id.is_empty() || id.len() > 256 || !seen.insert(id.clone()) {
                return Err(ApiError::invalid("账号 ID 无效或重复"));
            }
            // 保留已存条目时不阻止用户移除其他已删除账号
            if !old.account_ids.contains(id) {
                let account = upstream::runtime(host, id).await?;
                if account.provider_id != "openai" || account.authentication_kind != "api_key" {
                    return Err(ApiError::invalid("Cline 转换仅支持 OpenAI API Key 账号"));
                }
            }
        }
        input.account_ids.sort();
        let config = Configuration {
            account_ids: input.account_ids,
        };
        let version =
            state::put(host, NAMESPACE, "accounts", input.expected_version, &config).await?;
        Ok(self.view(Some(version), config))
    }
    pub async fn register(
        &self,
        _call: TypedCall<Empty>,
    ) -> std::result::Result<TypedReply<UpstreamAdapterRegistration>, PluginFault> {
        // 首次安装和配置切换在状态 fence 激活前注册，只能使用握手中的冻结配置
        let adapters = if self.active.is_empty() {
            Vec::new()
        } else {
            vec![UpstreamAdapterDeclaration {
                id: ADAPTER.into(),
                provider: BuiltinProvider::OpenAi,
                base_url: "https://api.cline.bot/api/v1/".into(),
                paths: vec![UpstreamPath {
                    path: "chat/completions".into(),
                    purpose: UpstreamPathPurpose::Inference,
                }],
                authentication_kinds: vec!["api_key".into()],
                transport: UpstreamTransport::HttpSse,
                protocol: "openai".into(),
                models: Vec::new(),
                account_ids: self.active.clone(),
            }]
        };
        Ok(TypedReply::new(UpstreamAdapterRegistration { adapters }))
    }
    pub async fn execute(
        &self,
        logs: Arc<upstream_pin::Runtime>,
        call: TypedCall<UpstreamAdapterRequest>,
    ) -> std::result::Result<TypedReply<Empty>, PluginFault> {
        if call.request.adapter_id != ADAPTER
            || call.request.provider != BuiltinProvider::OpenAi
            || call.request.protocol != "openai"
            || self.active.binary_search(&call.request.account_id).is_err()
        {
            return failure_reply(
                UpstreamFailureKind::InvalidRequest,
                400,
                "Cline 转换账号或协议不匹配",
            );
        }
        if call.request.continuation.is_some() || call.request.client_transport == "websocket" {
            return failure_reply(
                UpstreamFailureKind::Unsupported,
                400,
                "Cline 转换仅支持 HTTP 完整历史，不支持 WebSocket 或原生续接",
            );
        }
        let (mut body, stream) = match request::convert(&call.payload, &call.request.upstream_model)
        {
            Ok(value) => value,
            Err(message) => return failure_reply(UpstreamFailureKind::Unsupported, 400, message),
        };
        let stream = match call.request.client_transport.as_str() {
            "http_json" => false,
            "http_sse" => true,
            _ => stream,
        };
        let key = upstream_pin::adapter_request(
            &call.host,
            &logs,
            call.context
                .request_id
                .clone()
                .unwrap_or_else(|| format!("call-{}", call.context.call_id)),
            &call.request.account_id,
            &call.request.upstream_model,
            &mut body,
        )
        .await?;
        let mut headers = vec![
            ("content-type".into(), b"application/json".to_vec()),
            ("accept".into(), b"text/event-stream".to_vec()),
        ];
        // 仅传递已有业务标识头，不向 Cline 转发下游认证或其他站点 Cookie
        headers.extend(
            call.request
                .headers
                .iter()
                .filter(|(name, _)| {
                    ["user-agent", "http-referer", "x-title"]
                        .iter()
                        .any(|allowed| name.eq_ignore_ascii_case(allowed))
                })
                .cloned(),
        );
        let payload = serde_json::to_vec(&body)
            .map_err(|_| PluginFault::new(ErrorCode::InvalidInput, "Cline 请求编码失败"))?;
        let upstream = match call
            .host
            .upstream_http(
                UpstreamHttpRequest {
                    method: "POST".into(),
                    path: "chat/completions".into(),
                    headers,
                    query: Vec::new(),
                },
                payload,
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                logs.failed(&key);
                return Err(error);
            }
        };
        let json_body = upstream
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .is_some_and(|(_, value)| {
                String::from_utf8_lossy(value)
                    .to_ascii_lowercase()
                    .starts_with("application/json")
            });
        let mut observation = ResponseObservation::new("http_sse");
        observation.http_version = Some(upstream.http_version);
        observation.status_code = Some(upstream.status);
        observation.request_id = header(&upstream.headers, "x-request-id")
            .filter(|s| s.len() <= 512 && !s.chars().any(char::is_control));
        let retry_after_ms = header(&upstream.headers, "retry-after")
            .and_then(|v| v.parse::<u64>().ok())
            .and_then(|v| v.checked_mul(1000))
            .map(|delay| delay.min(24 * 60 * 60 * 1000));
        let maximum = self.maximum_stream_chunk_bytes;
        let parallel_tool_calls = body
            .get("parallel_tool_calls")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let source = Stream {
            body: upstream.body,
            response: response::Response::new(
                call.request.upstream_model,
                stream,
                parallel_tool_calls,
            ),
            sse: response::Sse::default(),
            frames: VecDeque::new(),
            json_body,
            status: upstream.status,
            observation: Some(observation),
            logs,
            key,
            ended: false,
            started_sent: false,
            maximum,
            retry_after_ms,
        };
        Ok(TypedReply::new(Empty {}).with_stream(ResponseStream::pull(Box::new(source))))
    }
}

fn header(headers: &[(String, Vec<u8>)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| String::from_utf8(v.clone()).ok())
}
fn failure(kind: UpstreamFailureKind, status: u16, message: &str) -> UpstreamAdapterEvent {
    let mut event = UpstreamAdapterEvent::new(ExecutionEvent::default());
    event.failure = Some(UpstreamFailure {
        kind,
        status: Some(status),
        message: message.into(),
        code: Some("cline_adapter_error".into()),
        retry_after_ms: None,
    });
    event
}
fn encode(event: UpstreamAdapterEvent) -> std::result::Result<Vec<u8>, PluginFault> {
    event
        .encode()
        .map_err(|_| PluginFault::new(ErrorCode::Capacity, "Cline 转换事件超过协议容量"))
}
fn failure_reply(
    kind: UpstreamFailureKind,
    status: u16,
    message: &str,
) -> std::result::Result<TypedReply<Empty>, PluginFault> {
    Ok(
        TypedReply::new(Empty {}).with_stream(ResponseStream::from_chunks(vec![encode(failure(
            kind, status, message,
        ))?])),
    )
}

fn http_failure(status: u16) -> UpstreamFailureKind {
    match status {
        400 | 404 | 422 => UpstreamFailureKind::InvalidRequest,
        401 => UpstreamFailureKind::Unauthorized,
        402 => UpstreamFailureKind::QuotaExhausted,
        403 => UpstreamFailureKind::PermissionDenied,
        429 => UpstreamFailureKind::RateLimited,
        408 | 504 => UpstreamFailureKind::Timeout,
        _ => UpstreamFailureKind::Unavailable,
    }
}

fn chat_failure(raw: &Value) -> Option<(UpstreamFailureKind, u16, &'static str)> {
    let value = if raw["success"] == true {
        &raw["data"]
    } else {
        raw
    };
    let choice = value.pointer("/choices/0");
    let error = value
        .get("error")
        .filter(|v| !v.is_null())
        .or_else(|| choice.and_then(|c| c.get("error")).filter(|v| !v.is_null()));
    if error.is_none()
        && raw["success"] != false
        && choice.is_none_or(|c| c["finish_reason"] != "error")
    {
        return None;
    }
    let code = error.and_then(|e| e.get("code"));
    let number = code.and_then(|c| {
        c.as_u64()
            .or_else(|| c.as_str().and_then(|s| s.parse::<u64>().ok()))
    });
    if let Some(status) = number.filter(|status| (400..600).contains(status)) {
        return Some((
            http_failure(status as u16),
            status as u16,
            "Cline 在响应流中拒绝请求",
        ));
    }
    Some(match code.and_then(Value::as_str) {
        Some("context_length_exceeded" | "invalid_request_error") => (
            UpstreamFailureKind::InvalidRequest,
            400,
            "Cline 拒绝请求：上下文超限或参数无效",
        ),
        Some("content_filter") => (
            UpstreamFailureKind::PermissionDenied,
            403,
            "Cline 内容过滤拒绝请求",
        ),
        Some("rate_limit" | "rate_limit_exceeded") => {
            (UpstreamFailureKind::RateLimited, 429, "Cline 上游限流")
        }
        Some("insufficient_quota" | "insufficient_credits") => (
            UpstreamFailureKind::QuotaExhausted,
            402,
            "Cline 上游额度不足",
        ),
        _ => (
            UpstreamFailureKind::Unavailable,
            502,
            "Cline 在响应流中返回上游错误",
        ),
    })
}

struct Stream {
    body: HostHttpBody,
    response: response::Response,
    sse: response::Sse,
    frames: VecDeque<response::Frame>,
    json_body: bool,
    status: u16,
    observation: Option<ResponseObservation>,
    logs: Arc<upstream_pin::Runtime>,
    key: (String, u32),
    ended: bool,
    started_sent: bool,
    maximum: usize,
    retry_after_ms: Option<u64>,
}
impl Stream {
    async fn fail(
        &mut self,
        kind: UpstreamFailureKind,
        status: u16,
        message: &str,
    ) -> std::result::Result<Vec<u8>, PluginFault> {
        self.ended = true;
        self.response.events.clear();
        self.frames.clear();
        self.logs.failed(&self.key);
        let _ = self.body.close().await;
        let mut event = failure(kind, status, message);
        if let Some(failure) = &mut event.failure {
            failure.retry_after_ms = self.retry_after_ms;
        }
        event.observation = self.observation.take();
        if self.response.usage != Usage::default() {
            self.response.events.push_back(event);
            let mut usage = ExecutionEvent::default();
            if !self.started_sent {
                usage.facts.push(self.response.started_fact());
                self.started_sent = true;
            }
            usage.facts.push(CanonicalEvent::Usage {
                usage: self.response.usage.clone(),
            });
            return encode(UpstreamAdapterEvent::new(usage));
        }
        encode(event)
    }
    fn observe(&mut self) {
        if let Some(metadata) = self.response.metadata_update.take() {
            self.logs.observe(
                &self.key,
                &json!({"provider_metadata":self.response.metadata}),
            );
            self.observation
                .get_or_insert_with(|| ResponseObservation::new("http_sse"))
                .provider_metadata = Some(metadata);
        }
        if let Some(tier) = self.response.service_tier_update.take() {
            self.observation
                .get_or_insert_with(|| ResponseObservation::new("http_sse"))
                .service_tier = Some(tier);
        }
    }
    fn finish(&mut self) -> std::result::Result<(), &'static str> {
        self.response.complete()?;
        self.ended = true;
        Ok(())
    }
    async fn next_event(&mut self) -> Option<std::result::Result<Vec<u8>, PluginFault>> {
        loop {
            if let Some(mut event) = self.response.events.pop_front() {
                event.observation = event.observation.take().or_else(|| self.observation.take());
                let starts = event
                    .event
                    .facts
                    .iter()
                    .any(|fact| matches!(fact, CanonicalEvent::Started { .. }));
                return Some(match encode(event) {
                    Ok(bytes) if bytes.len() <= self.maximum => {
                        self.started_sent |= starts;
                        Ok(bytes)
                    }
                    _ => {
                        self.fail(
                            UpstreamFailureKind::Protocol,
                            502,
                            "转换事件超过宿主流分块容量，请减少单次输出",
                        )
                        .await
                    }
                });
            }
            if self.ended {
                return None;
            }
            if let Some(frame) = self.frames.pop_front() {
                let result = if frame.error {
                    if let Ok(value) = serde_json::from_slice::<Value>(&frame.data)
                        && let Some((kind, status, message)) = chat_failure(&value)
                    {
                        return Some(self.fail(kind, status, message).await);
                    }
                    Err("Cline SSE 返回错误事件")
                } else if frame.data.trim_ascii() == b"[DONE]" {
                    if self.frames.is_empty() {
                        self.finish()
                    } else {
                        Err("Chat 在 DONE 后继续返回事件")
                    }
                } else {
                    match serde_json::from_slice::<Value>(&frame.data) {
                        Err(_) => Err("Cline SSE data 不是有效 JSON"),
                        Ok(value) => {
                            if let Some((kind, status, message)) = chat_failure(&value) {
                                return Some(self.fail(kind, status, message).await);
                            }
                            self.response.consume(&value, false)
                        }
                    }
                };
                self.observe();
                if let Err(message) = result {
                    return Some(self.fail(UpstreamFailureKind::Protocol, 502, message).await);
                }
                if self.ended {
                    let _ = self.body.close().await;
                }
                continue;
            }
            if !(200..300).contains(&self.status) {
                let kind = http_failure(self.status);
                let status = if (400..600).contains(&self.status) {
                    self.status
                } else {
                    502
                };
                return Some(
                    self.fail(
                        kind,
                        status,
                        "Cline 上游拒绝请求，请检查账号额度、模型和请求参数",
                    )
                    .await,
                );
            }
            if self.json_body {
                let mut bytes = Vec::new();
                loop {
                    match self.body.read().await {
                        Ok(Some(chunk)) if bytes.len() + chunk.len() <= response::MAX_BODY => {
                            bytes.extend(chunk)
                        }
                        Ok(None) => break,
                        Ok(Some(_)) => {
                            return Some(
                                self.fail(
                                    UpstreamFailureKind::Protocol,
                                    502,
                                    "Cline JSON 响应超过 512 KiB",
                                )
                                .await,
                            );
                        }
                        Err(error) => return Some(self.network_error(error).await),
                    }
                }
                let value = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        return Some(
                            self.fail(
                                UpstreamFailureKind::Protocol,
                                502,
                                "Cline 响应不是有效 JSON",
                            )
                            .await,
                        );
                    }
                };
                if let Some((kind, status, message)) = chat_failure(&value) {
                    return Some(self.fail(kind, status, message).await);
                }
                let result = self
                    .response
                    .consume(&value, true)
                    .and_then(|()| self.finish());
                self.observe();
                if let Err(message) = result {
                    return Some(self.fail(UpstreamFailureKind::Protocol, 502, message).await);
                }
                continue;
            }
            match self.body.read().await {
                Err(error) => return Some(self.network_error(error).await),
                Ok(None) => {
                    let result = self.sse.finish().and_then(|()| self.finish());
                    if let Err(message) = result {
                        return Some(self.fail(UpstreamFailureKind::Protocol, 502, message).await);
                    }
                }
                Ok(Some(chunk)) => {
                    let frames = match self.sse.push(&chunk) {
                        Ok(frames) => frames,
                        Err(message) => {
                            return Some(
                                self.fail(UpstreamFailureKind::Protocol, 502, message).await,
                            );
                        }
                    };
                    self.frames.extend(frames);
                }
            }
        }
    }
    async fn network_error(
        &mut self,
        error: PluginFault,
    ) -> std::result::Result<Vec<u8>, PluginFault> {
        if error.code == ErrorCode::Cancelled {
            self.ended = true;
            self.logs.failed(&self.key);
            return Err(error);
        }
        let kind = if error.code == ErrorCode::Timeout {
            UpstreamFailureKind::Timeout
        } else {
            UpstreamFailureKind::Unavailable
        };
        self.fail(
            kind,
            if error.code == ErrorCode::Timeout {
                504
            } else {
                502
            },
            "Cline 上游读取中断，未生成成功终态",
        )
        .await
    }
}
impl PullResponseStream for Stream {
    fn next(&mut self) -> PullResponseFuture<'_> {
        Box::pin(self.next_event())
    }
}
