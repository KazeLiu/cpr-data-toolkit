//! Chat 增量归一化；同一份输出同时生成 Responses wire 与宿主标准事实

use gateway_plugin_sdk::call::{
    model::{
        CanonicalEvent as Fact, ContentKind, ExecutionEvent, FinishReason, Usage, WireEvent,
        WirePayload,
    },
    upstream_adapter::UpstreamAdapterEvent,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, &'static str>;
pub const MAX_BODY: usize = 512 * 1024;

enum Item {
    Text {
        id: String,
        text: String,
        refusal: bool,
    },
    Reasoning {
        id: String,
        text: String,
    },
    Tool {
        id: String,
        call_id: String,
        name: String,
        arguments: String,
    },
}
impl Item {
    fn id(&self) -> &str {
        match self {
            Self::Text { id, .. } | Self::Reasoning { id, .. } | Self::Tool { id, .. } => id,
        }
    }
    fn value(&self, status: &str) -> Value {
        match self {
            Self::Text { id, text, refusal } => {
                json!({"type":"message","id":id,"role":"assistant","status":status,
                "content":[if *refusal { json!({"type":"refusal","refusal":text}) }
                    else { json!({"type":"output_text","text":text,"annotations":[]}) }]})
            }
            Self::Reasoning { id, text } => {
                json!({"type":"reasoning","id":id,"summary":[{"type":"summary_text","text":text}],"content":[{"type":"reasoning_text","text":text}]})
            }
            Self::Tool {
                id,
                call_id,
                name,
                arguments,
            } => {
                json!({"type":"function_call","id":id,"call_id":call_id,"name":name,"arguments":arguments,"status":status})
            }
        }
    }
}

#[derive(Default)]
struct Tool {
    id: String,
    name: String,
    arguments: String,
    output: Option<usize>,
}

pub struct Response {
    id: String,
    model: String,
    created: i64,
    sequence: u64,
    stream: bool,
    parallel_tool_calls: bool,
    started: bool,
    items: Vec<Item>,
    text: Option<usize>,
    reasoning: Option<usize>,
    tools: BTreeMap<u64, Tool>,
    finish: Option<FinishReason>,
    pub usage: Usage,
    size: usize,
    pub metadata: Option<Value>,
    pub service_tier: Option<String>,
    pub metadata_update: Option<String>,
    pub service_tier_update: Option<String>,
    pub events: VecDeque<UpstreamAdapterEvent>,
}

impl Response {
    pub fn started_fact(&self) -> Fact {
        Fact::Started {
            id: self.id.clone(),
            model: Some(self.model.clone()),
        }
    }
    pub fn new(model: String, stream: bool, parallel_tool_calls: bool) -> Self {
        Self {
            id: format!("resp_{}", uuid::Uuid::new_v4().simple()),
            model,
            created: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_secs() as i64,
            sequence: 0,
            stream,
            parallel_tool_calls,
            started: false,
            items: Vec::new(),
            text: None,
            reasoning: None,
            tools: BTreeMap::new(),
            finish: None,
            usage: Usage::default(),
            size: 0,
            metadata: None,
            service_tier: None,
            metadata_update: None,
            service_tier_update: None,
            events: VecDeque::new(),
        }
    }
    fn push(&mut self, kind: &str, mut data: Value, fact: Option<Fact>) {
        let mut event = ExecutionEvent::default();
        if self.stream {
            data["type"] = json!(kind);
            data["sequence_number"] = json!(self.sequence);
            self.sequence += 1;
            event.wire = Some(WireEvent {
                protocol: "openai".into(),
                payload: WirePayload::Json {
                    event: Some(kind.into()),
                    data,
                    id: None,
                    retry: None,
                    raw_sse: None,
                },
            });
        }
        if let Some(fact) = fact {
            event.facts.push(fact);
        }
        if event.wire.is_some() || !event.facts.is_empty() {
            self.events.push_back(UpstreamAdapterEvent::new(event));
        }
    }
    fn value(&self, status: &str) -> Value {
        let incomplete = match self.finish {
            Some(FinishReason::Length) => json!({"reason":"max_output_tokens"}),
            Some(FinishReason::ContentFilter) => json!({"reason":"content_filter"}),
            _ => Value::Null,
        };
        let mut value = json!({"id":self.id,"object":"response","created_at":self.created,"status":status,
            "error":null,"incomplete_details":incomplete,"model":self.model,"output":self.items.iter().map(|i|i.value(status)).collect::<Vec<_>>(),
            "parallel_tool_calls":self.parallel_tool_calls,"store":false,"usage":null});
        if status != "in_progress" && self.usage != Usage::default() {
            value["usage"] = json!({"input_tokens":self.usage.input_tokens,"output_tokens":self.usage.output_tokens,
                "total_tokens":self.usage.total_tokens,"input_tokens_details":{"cached_tokens":self.usage.cached_tokens},
                "output_tokens_details":{"reasoning_tokens":self.usage.reasoning_tokens}});
        }
        if let Some(metadata) = &self.metadata {
            value["provider_metadata"] = metadata.clone();
        }
        if let Some(tier) = &self.service_tier {
            value["service_tier"] = json!(tier);
        }
        value
    }
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        self.push(
            "response.created",
            json!({"response":self.value("in_progress")}),
            Some(Fact::Started {
                id: self.id.clone(),
                model: Some(self.model.clone()),
            }),
        );
        self.push(
            "response.in_progress",
            json!({"response":self.value("in_progress")}),
            None,
        );
    }
    fn add(&mut self, item: Item, kind: ContentKind) -> usize {
        let index = self.items.len();
        let mut value = item.value("in_progress");
        match kind {
            ContentKind::Text => value["content"] = json!([]),
            ContentKind::Reasoning => {
                value["summary"] = json!([]);
                value["content"] = json!([]);
            }
            _ => {}
        }
        self.items.push(item);
        self.push(
            "response.output_item.added",
            json!({"output_index":index,"item":value}),
            Some(Fact::ContentAdded {
                index: index as u32,
                kind,
            }),
        );
        index
    }
    fn append(&mut self, text: &str, reasoning: bool, refusal: bool) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        if self.finish.is_some() {
            return Err("Chat 在 finish_reason 后继续返回内容");
        }
        self.size = self
            .size
            .checked_add(text.len())
            .ok_or("输出超过容量限制")?;
        if self.size > MAX_BODY {
            return Err("输出超过 512 KiB 转换容量");
        }
        let existing = if reasoning { self.reasoning } else { self.text };
        let index = match existing {
            Some(index) => index,
            None => {
                let id = format!(
                    "{}_{}",
                    if reasoning { "rs" } else { "msg" },
                    uuid::Uuid::new_v4().simple()
                );
                let item = if reasoning {
                    Item::Reasoning {
                        id,
                        text: String::new(),
                    }
                } else {
                    Item::Text {
                        id,
                        text: String::new(),
                        refusal,
                    }
                };
                let index = self.add(
                    item,
                    if reasoning {
                        ContentKind::Reasoning
                    } else {
                        ContentKind::Text
                    },
                );
                if reasoning {
                    self.reasoning = Some(index);
                } else {
                    self.text = Some(index);
                }
                let kind = if reasoning {
                    "response.reasoning_summary_part.added"
                } else {
                    "response.content_part.added"
                };
                let part = if reasoning {
                    json!({"type":"summary_text","text":""})
                } else if refusal {
                    json!({"type":"refusal","refusal":""})
                } else {
                    json!({"type":"output_text","text":"","annotations":[]})
                };
                let mut data =
                    json!({"item_id":self.items[index].id(),"output_index":index,"part":part});
                data[if reasoning {
                    "summary_index"
                } else {
                    "content_index"
                }] = json!(0);
                self.push(kind, data, None);
                index
            }
        };
        let item = &mut self.items[index];
        match item {
            Item::Text {
                text: target,
                refusal: was,
                ..
            } => {
                if *was != refusal {
                    return Err("同一 Chat 消息混合了拒绝和文本内容");
                }
                target.push_str(text);
            }
            Item::Reasoning { text: target, .. } => target.push_str(text),
            _ => unreachable!("text index"),
        }
        let fact = if reasoning {
            Fact::ReasoningDelta {
                index: index as u32,
                text: text.into(),
            }
        } else {
            Fact::TextDelta {
                index: index as u32,
                text: text.into(),
            }
        };
        let kind = if reasoning {
            "response.reasoning_summary_text.delta"
        } else if refusal {
            "response.refusal.delta"
        } else {
            "response.output_text.delta"
        };
        let mut data = json!({"item_id":self.items[index].id(),"output_index":index,"delta":text});
        data[if reasoning {
            "summary_index"
        } else {
            "content_index"
        }] = json!(0);
        self.push(kind, data, Some(fact));
        Ok(())
    }
    fn tool(&mut self, value: &Value, fallback_index: u64, json_response: bool) -> Result<()> {
        if self.finish.is_some() {
            return Err("Chat 在 finish_reason 后继续返回工具调用");
        }
        if value
            .get("type")
            .is_some_and(|v| !v.is_null() && v != "function")
        {
            return Err("Chat 返回了非 function 工具");
        }
        let key = match value.get("index") {
            Some(index) => index.as_u64().ok_or("工具 index 须为非负整数")?,
            None if json_response => fallback_index,
            None => return Err("Chat 工具增量缺少 index"),
        };
        if !self.tools.contains_key(&key) && self.tools.len() >= 128 {
            return Err("单次工具调用超过 128 项");
        }
        let mut tool = self.tools.remove(&key).unwrap_or_default();
        if let Some(id) = value.get("id").filter(|v| !v.is_null()) {
            let id = id.as_str().ok_or("工具调用 ID 须为字符串")?;
            if id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
                return Err("工具调用 ID 无效");
            }
            if !tool.id.is_empty() && tool.id != id {
                return Err("工具调用 ID 在流中发生变化");
            }
            tool.id = id.into();
        }
        let function = value.get("function").unwrap_or(&Value::Null);
        if !function.is_null() && !function.is_object() {
            return Err("工具 function 须为对象");
        }
        if let Some(name) = function.get("name").filter(|v| !v.is_null()) {
            let name = name.as_str().ok_or("工具名称须为字符串")?;
            if tool.output.is_none() {
                // 发布前保留名称分片；发布后只接受同一名称的重复回报
                tool.name.push_str(name);
            } else if !name.is_empty() && name != tool.name {
                return Err("工具名称在参数开始后继续变化");
            }
            if tool.name.len() > 512 || tool.name.chars().any(char::is_control) {
                return Err("工具名称无效");
            }
        }
        let arguments = match function.get("arguments").filter(|v| !v.is_null()) {
            Some(value) => value.as_str().ok_or("工具参数须为 JSON 字符串")?,
            None => "",
        };
        if tool.output.is_none() {
            tool.arguments.push_str(arguments);
        }
        self.size += arguments.len();
        if self.size > MAX_BODY {
            return Err("输出超过 512 KiB 转换容量");
        }
        if !tool.id.is_empty()
            && !tool.name.is_empty()
            && (!arguments.is_empty() || !tool.arguments.is_empty())
        {
            let first = tool.output.is_none();
            let delta = if first {
                std::mem::take(&mut tool.arguments)
            } else {
                arguments.to_owned()
            };
            let index = match tool.output {
                Some(index) => {
                    let Item::Tool {
                        arguments: target, ..
                    } = &mut self.items[index]
                    else {
                        unreachable!("tool index")
                    };
                    target.push_str(&delta);
                    index
                }
                None => {
                    let index = self.add(
                        Item::Tool {
                            id: format!("fc_{}", uuid::Uuid::new_v4().simple()),
                            call_id: tool.id.clone(),
                            name: tool.name.clone(),
                            arguments: String::new(),
                        },
                        ContentKind::ToolCall,
                    );
                    tool.output = Some(index);
                    let Item::Tool {
                        arguments: target, ..
                    } = &mut self.items[index]
                    else {
                        unreachable!("tool index")
                    };
                    target.push_str(&delta);
                    index
                }
            };
            // 在首次工具增量中同时提交名称和已缓冲参数
            self.push(
                "response.function_call_arguments.delta",
                json!({"item_id":self.items[index].id(),"output_index":index,"delta":delta}),
                Some(Fact::ToolCallDelta {
                    index: index as u32,
                    id: tool.id.clone(),
                    name: first.then(|| tool.name.clone()),
                    arguments: delta,
                }),
            );
        }
        if !tool.id.is_empty() && self.tools.values().any(|other| other.id == tool.id) {
            return Err("Chat 返回了重复工具调用 ID");
        }
        self.tools.insert(key, tool);
        Ok(())
    }
    pub fn consume(&mut self, raw: &Value, json_response: bool) -> Result<()> {
        let value = if raw.get("success") == Some(&json!(true)) {
            raw.get("data").ok_or("Cline 成功信封缺少 data")?
        } else {
            raw
        };
        if raw.get("success") == Some(&json!(false))
            || value.get("error").is_some_and(|v| !v.is_null())
        {
            return Err("Cline 在成功 HTTP 响应中返回错误");
        }
        self.start();
        if let Some(tier) = value.get("service_tier").and_then(Value::as_str) {
            if tier.len() > 128 || tier.chars().any(char::is_control) {
                return Err("service_tier 无效");
            }
            if self.service_tier.as_deref() != Some(tier) {
                self.service_tier = Some(tier.into());
                self.service_tier_update = Some(tier.into());
            }
        }
        self.observe_metadata(value)?;
        if let Some(usage) = value.get("usage").filter(|v| !v.is_null()) {
            let incoming = usage_value(usage)?;
            for (target, source) in [
                (&mut self.usage.input_tokens, incoming.input_tokens),
                (&mut self.usage.output_tokens, incoming.output_tokens),
                (&mut self.usage.cached_tokens, incoming.cached_tokens),
                (
                    &mut self.usage.cache_write_tokens,
                    incoming.cache_write_tokens,
                ),
                (&mut self.usage.reasoning_tokens, incoming.reasoning_tokens),
                (&mut self.usage.total_tokens, incoming.total_tokens),
            ] {
                if source.is_some() {
                    *target = source;
                }
            }
            if incoming.total_tokens.is_none()
                && (incoming.input_tokens.is_some() || incoming.output_tokens.is_some())
                && let Some((input, output)) = self.usage.input_tokens.zip(self.usage.output_tokens)
            {
                self.usage.total_tokens = input
                    .checked_add(output)
                    .filter(|total| *total <= i64::MAX as u64);
            }
        }
        let choices = match value.get("choices") {
            None | Some(Value::Null)
                if !json_response && value.get("usage").is_some_and(Value::is_object) =>
            {
                return Ok(());
            }
            Some(Value::Array(choices)) => choices,
            _ => return Err("Chat 响应缺少 choices"),
        };
        if choices.len() > 1 {
            return Err("不支持多候选 Chat 响应");
        }
        for choice in choices {
            if choice
                .get("index")
                .and_then(Value::as_u64)
                .is_some_and(|v| v != 0)
            {
                return Err("Chat 候选序号不是 0");
            }
            let delta = choice
                .get(if json_response { "message" } else { "delta" })
                .unwrap_or(&Value::Null);
            if !delta.is_object()
                && !(delta.is_null()
                    && !json_response
                    && choice.get("finish_reason").is_some_and(|v| !v.is_null()))
            {
                return Err("Chat message / delta 须为对象");
            }
            self.observe_metadata(delta)?;
            for (field, reasoning, refusal) in [
                ("reasoning_content", true, false),
                ("reasoning", true, false),
                ("content", false, false),
                ("refusal", false, true),
            ] {
                if field == "reasoning"
                    && delta.get("reasoning_content").is_some_and(|v| !v.is_null())
                {
                    continue;
                }
                if let Some(text) = delta.get(field).filter(|v| !v.is_null()) {
                    self.append(
                        text.as_str().ok_or("Chat 内容增量须为字符串")?,
                        reasoning,
                        refusal,
                    )?;
                }
            }
            if let Some(calls) = delta.get("tool_calls").filter(|v| !v.is_null()) {
                for (index, call) in calls
                    .as_array()
                    .ok_or("tool_calls 须为数组")?
                    .iter()
                    .enumerate()
                {
                    self.tool(call, index as u64, json_response)?;
                }
            }
            if let Some(reason) = choice.get("finish_reason").filter(|v| !v.is_null()) {
                let reason = match reason.as_str() {
                    Some("stop") => FinishReason::Stop,
                    Some("length") => FinishReason::Length,
                    Some("tool_calls" | "function_call") => FinishReason::ToolCall,
                    Some("content_filter") => FinishReason::ContentFilter,
                    _ => return Err("Chat finish_reason 表示失败或无法识别的终态"),
                };
                // 重复结束原因不代表再次完成响应，Responses 终态统一由 complete 生成
                if self.finish.is_some_and(|previous| previous != reason) {
                    return Err("Chat 返回了冲突的 finish_reason");
                }
                self.finish = Some(reason);
            }
        }
        Ok(())
    }
    fn observe_metadata(&mut self, value: &Value) -> Result<()> {
        if let Some(metadata) = value
            .get("provider_metadata")
            .filter(|metadata| !metadata.is_null())
            .or_else(|| value.get("providerMetadata"))
            .filter(|metadata| !metadata.is_null() && self.metadata.as_ref() != Some(*metadata))
        {
            let encoded = serde_json::to_string(metadata).map_err(|_| "元数据编码失败")?;
            if encoded.len() > 32 * 1024 {
                return Err("上游路由元数据超过容量");
            }
            self.metadata = Some(metadata.clone());
            self.metadata_update = Some(encoded);
        }
        Ok(())
    }
    pub fn complete(&mut self) -> Result<()> {
        let reason = self
            .finish
            .ok_or("Chat 流在 finish_reason 前结束，未生成成功终态")?;
        for (_, tool) in std::mem::take(&mut self.tools) {
            if tool.id.is_empty() || tool.name.is_empty() {
                return Err("Chat 工具调用缺少 ID 或名称");
            }
            if tool.output.is_none() {
                let index = self.add(
                    Item::Tool {
                        id: format!("fc_{}", uuid::Uuid::new_v4().simple()),
                        call_id: tool.id.clone(),
                        name: tool.name.clone(),
                        arguments: tool.arguments.clone(),
                    },
                    ContentKind::ToolCall,
                );
                self.push("response.function_call_arguments.delta",json!({"item_id":self.items[index].id(),"output_index":index,"delta":tool.arguments}),
                    Some(Fact::ToolCallDelta{index:index as u32,id:tool.id,name:Some(tool.name),arguments:tool.arguments}));
            }
        }
        for index in 0..self.items.len() {
            let item = &self.items[index];
            let id = item.id().to_owned();
            let value = item.value(
                if matches!(reason, FinishReason::Length | FinishReason::ContentFilter) {
                    "incomplete"
                } else {
                    "completed"
                },
            );
            let (kind, mut data) = match item {
                Item::Text { text, refusal, .. } => (
                    if *refusal {
                        "response.refusal.done"
                    } else {
                        "response.output_text.done"
                    },
                    if *refusal {
                        json!({"refusal":text,"content_index":0})
                    } else {
                        json!({"text":text,"content_index":0})
                    },
                ),
                Item::Reasoning { text, .. } => (
                    "response.reasoning_summary_text.done",
                    json!({"text":text,"summary_index":0}),
                ),
                Item::Tool { arguments, .. } => (
                    "response.function_call_arguments.done",
                    json!({"arguments":arguments}),
                ),
            };
            data["item_id"] = json!(id);
            data["output_index"] = json!(index);
            self.push(kind, data, None);
            if value["type"] == "message" {
                self.push("response.content_part.done",json!({"item_id":id,"output_index":index,"content_index":0,"part":value["content"][0]}),None);
            } else if value["type"] == "reasoning" {
                self.push("response.reasoning_summary_part.done",json!({"item_id":id,"output_index":index,"summary_index":0,"part":value["summary"][0]}),None);
            }
            self.push(
                "response.output_item.done",
                json!({"output_index":index,"item":value}),
                None,
            );
        }
        self.events
            .push_back(UpstreamAdapterEvent::new(ExecutionEvent::canonical(
                Fact::Usage {
                    usage: self.usage.clone(),
                },
            )));
        let status = if matches!(reason, FinishReason::Length | FinishReason::ContentFilter) {
            "incomplete"
        } else {
            "completed"
        };
        let response = self.value(status);
        let completed = Fact::Completed {
            id: self.id.clone(),
            model: Some(self.model.clone()),
            reason,
        };
        if self.stream {
            self.push(
                if status == "completed" {
                    "response.completed"
                } else {
                    "response.incomplete"
                },
                json!({"response":response}),
                Some(completed),
            );
        } else {
            self.events.push_back(UpstreamAdapterEvent::new(
                ExecutionEvent::wire(WireEvent {
                    protocol: "openai".into(),
                    payload: WirePayload::RawJson {
                        body: serde_json::to_vec(&response).map_err(|_| "Responses 编码失败")?,
                    },
                })
                .with_fact(completed),
            ));
        }
        Ok(())
    }
}

fn usage_value(value: &Value) -> Result<Usage> {
    if !value.is_object() {
        return Err("Chat usage 须为对象");
    }
    fn count(value: &Value) -> Result<Option<u64>> {
        if value.is_null() {
            return Ok(None);
        }
        value
            .as_u64()
            .filter(|v| *v <= i64::MAX as u64)
            .map(Some)
            .ok_or("Token 用量须为非负整数")
    }
    let usage = Usage {
        input_tokens: count(&value["prompt_tokens"])?,
        output_tokens: count(&value["completion_tokens"])?,
        cached_tokens: count(
            value
                .pointer("/prompt_tokens_details/cached_tokens")
                .filter(|v| !v.is_null())
                .unwrap_or(&value["prompt_cache_hit_tokens"]),
        )?,
        reasoning_tokens: count(&value["completion_tokens_details"]["reasoning_tokens"])?,
        cache_write_tokens: count(&value["prompt_tokens_details"]["cache_write_tokens"])?,
        total_tokens: count(&value["total_tokens"])?,
        ..Usage::default()
    };
    Ok(usage)
}

/// SSE 按行累积，支持跨网络块的 UTF-8、CRLF、LF 和多行 data
#[derive(Default)]
pub struct Sse {
    pending: Vec<u8>,
    data: Vec<u8>,
    frame_bytes: usize,
    skip_lf: bool,
    scanned: usize,
    error_event: bool,
}
pub struct Frame {
    pub data: Vec<u8>,
    pub error: bool,
}
impl Sse {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Frame>> {
        let bytes = if self.skip_lf && !bytes.is_empty() {
            self.skip_lf = false;
            bytes.strip_prefix(b"\n").unwrap_or(bytes)
        } else {
            bytes
        };
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut start = 0;
        let mut scan = self.scanned;
        while let Some(relative) = self.pending[scan..]
            .iter()
            .position(|b| matches!(*b, b'\r' | b'\n'))
        {
            let end = scan + relative;
            self.skip_lf = self.pending[end] == b'\r' && end + 1 == self.pending.len();
            let next = end
                + if self.pending[end] == b'\r' && self.pending.get(end + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
            let line = &self.pending[start..end];
            self.frame_bytes += next - start;
            if self.frame_bytes > MAX_BODY {
                return Err("单个 SSE 帧超过 512 KiB");
            }
            if line.is_empty() {
                if !self.data.is_empty() || self.error_event {
                    self.data.pop();
                    events.push(Frame {
                        data: std::mem::take(&mut self.data),
                        error: self.error_event,
                    });
                }
                self.frame_bytes = 0;
                self.error_event = false;
            } else if let Some(event) = line.strip_prefix(b"event:") {
                self.error_event = matches!(
                    event.trim_ascii(),
                    b"error" | b"response.failed" | b"response.error"
                );
            } else if let Some(data) = line.strip_prefix(b"data:") {
                self.data
                    .extend_from_slice(data.strip_prefix(b" ").unwrap_or(data));
                self.data.push(b'\n');
            }
            start = next;
            scan = next;
        }
        self.pending.drain(..start);
        self.scanned = self.pending.len();
        if self.pending.len() + self.frame_bytes > MAX_BODY {
            return Err("单个 SSE 帧超过 512 KiB");
        }
        Ok(events)
    }
    pub fn finish(&self) -> Result<()> {
        if !self.pending.is_empty() || !self.data.is_empty() || self.error_event {
            Err("SSE 在完整帧结束前断开")
        } else {
            Ok(())
        }
    }
}
