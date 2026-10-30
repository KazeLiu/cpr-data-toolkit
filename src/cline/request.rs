//! Responses 无状态完整历史转为 Chat；不能表达的输入在发送前拒绝

use serde_json::{Map, Value, json};

pub type Result<T> = std::result::Result<T, &'static str>;

fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or("请求字段格式不正确")
}

fn content(value: &Value, images: bool) -> Result<Value> {
    if value.is_string() {
        return Ok(value.clone());
    }
    let parts = value.as_array().ok_or("消息内容须为文本或内容数组")?;
    let mut output = Vec::with_capacity(parts.len());
    for part in parts {
        match string(part, "type")? {
            "input_text" | "output_text" => {
                output.push(json!({"type":"text", "text":string(part, "text")?}));
            }
            "refusal" if !images => {
                output.push(json!({"type":"text", "text":string(part, "refusal")?}));
            }
            "input_image" if images => {
                if part.get("file_id").is_some_and(|v| !v.is_null()) {
                    return Err("图片不支持 file_id，请使用图片 URL");
                }
                let image = string(part, "image_url")?;
                if !(image.starts_with("https://")
                    || image.starts_with("http://")
                    || image.starts_with("data:image/"))
                {
                    return Err("图片仅支持 HTTP(S) URL 或 data:image，不支持 file_id");
                }
                let mut image_url = json!({"url":image});
                if let Some(detail) = part.get("detail") {
                    image_url["detail"] = detail.clone();
                }
                output.push(json!({"type":"image_url", "image_url":image_url}));
            }
            _ => return Err("Cline 转换不支持此消息内容类型，请使用文本或图片 URL"),
        }
    }
    // 统一首次字符串输入与后续回灌的文本数组，保持上游历史前缀形态稳定
    if output.iter().all(|part| part["type"] == "text") {
        let mut text = String::new();
        for part in &output {
            text.push_str(part["text"].as_str().expect("converted text part"));
        }
        Ok(Value::String(text))
    } else {
        Ok(Value::Array(output))
    }
}

fn assistant(messages: &mut Vec<Value>, message: Value) {
    // Responses 将消息和函数调用分成独立 item，Chat 同一轮须合并工具调用
    if let Some(last) = messages.last_mut().filter(|last| {
        last["role"] == "assistant"
            && (last.get("tool_calls").is_some() || message.get("tool_calls").is_some())
    }) {
        if let Some(text) = message.get("content").and_then(Value::as_str) {
            match last.get_mut("content") {
                Some(Value::String(content)) => content.push_str(text),
                _ => last["content"] = json!(text),
            }
        }
        if let Some(reasoning) = message.get("reasoning_content").and_then(Value::as_str) {
            let text = last
                .get("reasoning_content")
                .and_then(Value::as_str)
                .unwrap_or("");
            last["reasoning_content"] = json!(format!("{text}{reasoning}"));
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            let target = last
                .as_object_mut()
                .expect("constructed message")
                .entry("tool_calls")
                .or_insert_with(|| json!([]));
            target
                .as_array_mut()
                .expect("constructed calls")
                .extend(calls.iter().cloned());
        }
        return;
    }
    messages.push(message);
}

pub fn convert(bytes: &[u8], model: &str) -> Result<(Map<String, Value>, bool)> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "Responses 正文不是有效 JSON")?;
    let root = value.as_object().ok_or("Responses 正文须为对象")?;
    if root
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
        || root.get("conversation").is_some_and(|v| !v.is_null())
    {
        return Err(
            "Cline 转换需要完整 input 历史，不支持 previous_response_id 或 conversation 续接",
        );
    }
    if root.get("background") == Some(&Value::Bool(true))
        || root.get("store") == Some(&Value::Bool(true))
    {
        return Err("Cline 转换不提供后台任务或响应存储，请设置 background=false、store=false");
    }
    for field in [
        "prompt",
        "context_management",
        "truncation",
        "max_tool_calls",
        "top_logprobs",
    ] {
        if root
            .get(field)
            .is_some_and(|v| !v.is_null() && v != "disabled")
        {
            return Err("Cline 转换不支持模板、压缩、自动截断、原生工具限制或 logprobs");
        }
    }
    let stream = match root.get("stream") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err("stream 须为布尔值"),
    };
    let mut messages = Vec::new();
    let mut reasoning = String::new();
    if let Some(instructions) = root.get("instructions").filter(|v| !v.is_null()) {
        messages.push(json!({"role":"system", "content":instructions.as_str().ok_or("instructions 须为文本")?}));
    }
    match root.get("input").ok_or("缺少 input")? {
        Value::String(text) => messages.push(json!({"role":"user", "content":text})),
        Value::Array(items) => {
            for item in items {
                let kind = match item.get("type") {
                    None => "message",
                    Some(value) => value.as_str().ok_or("input item 的 type 须为字符串")?,
                };
                match kind {
                    "message" => {
                        let role = string(item, "role")?;
                        if !matches!(role, "user" | "assistant" | "system" | "developer") {
                            return Err("消息 role 不受支持");
                        }
                        let role = if role == "developer" { "system" } else { role };
                        let mut message = json!({"role":role, "content":content(item.get("content").ok_or("缺少消息内容")?, role == "user")?});
                        if role == "assistant" && !reasoning.is_empty() {
                            message["reasoning_content"] = json!(std::mem::take(&mut reasoning));
                        }
                        if role == "assistant" {
                            assistant(&mut messages, message);
                        } else {
                            messages.push(message);
                        }
                    }
                    "function_call" => {
                        let call = json!({"id":string(item,"call_id")?, "type":"function", "function":{
                            "name":string(item,"name")?, "arguments":string(item,"arguments")?
                        }});
                        let mut message =
                            json!({"role":"assistant", "content":null, "tool_calls":[call]});
                        if !reasoning.is_empty() {
                            message["reasoning_content"] = json!(std::mem::take(&mut reasoning));
                        }
                        assistant(&mut messages, message);
                    }
                    "function_call_output" => {
                        let output = item.get("output").ok_or("缺少函数返回值")?;
                        messages.push(json!({"role":"tool", "tool_call_id":string(item,"call_id")?, "content":content(output, false)?}));
                    }
                    "reasoning" => {
                        if item.get("encrypted_content").is_some_and(|v| !v.is_null()) {
                            return Err(
                                "Cline 转换不能复用 Responses 的加密推理，请发送未加密完整历史",
                            );
                        }
                        // 仅回灌明确的完整推理文本，不把展示摘要冒充完整推理
                        if let Some(parts) = item.get("content").filter(|v| !v.is_null()) {
                            for part in parts.as_array().ok_or("推理 content 须为数组")? {
                                if string(part, "type")? != "reasoning_text" {
                                    return Err("推理历史内容类型不受支持");
                                }
                                reasoning.push_str(string(part, "text")?);
                            }
                        }
                    }
                    _ => {
                        return Err(
                            "Cline 转换仅支持消息、函数调用与结果，不支持原生工具、文件或 item 引用",
                        );
                    }
                }
            }
        }
        _ => return Err("input 须为文本或完整历史数组"),
    }
    if !reasoning.is_empty() {
        return Err("完整推理历史后缺少对应的 assistant 消息或函数调用");
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    body.insert("messages".into(), json!(messages));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({"include_usage":true}));
    // 上游统一使用 SSE，客户端是否流式由返回的 Responses wire 决定
    for name in [
        "temperature",
        "top_p",
        "parallel_tool_calls",
        "service_tier",
        "user",
        "provider",
        "providerOptions",
    ] {
        if let Some(value) = root.get(name).filter(|v| !v.is_null()) {
            body.insert(name.into(), value.clone());
        }
    }
    if let Some(value) = root.get("max_output_tokens").filter(|v| !v.is_null()) {
        if value.as_u64().is_none_or(|v| v == 0) {
            return Err("max_output_tokens 须为正整数");
        }
        body.insert("max_tokens".into(), value.clone());
    }
    if let Some(reasoning) = root.get("reasoning").filter(|v| !v.is_null()) {
        if !reasoning.is_object() {
            return Err("reasoning 须为对象");
        }
        if let Some(effort) = reasoning.get("effort") {
            body.insert("reasoning_effort".into(), effort.clone());
        }
    }
    if let Some(tools) = root.get("tools").filter(|v| !v.is_null()) {
        let mut converted = Vec::new();
        for tool in tools.as_array().ok_or("tools 须为数组")? {
            if string(tool, "type")? != "function" {
                return Err("仅支持 function 工具，不支持 OpenAI 原生或 custom 工具");
            }
            let mut function = tool.as_object().ok_or("工具格式不正确")?.clone();
            function.remove("type");
            string(tool, "name")?;
            converted.push(json!({"type":"function", "function":function}));
        }
        body.insert("tools".into(), json!(converted));
    }
    if let Some(choice) = root.get("tool_choice").filter(|v| !v.is_null()) {
        let choice = if choice.is_string() {
            match choice.as_str() {
                Some("auto" | "none" | "required") => choice.clone(),
                _ => return Err("tool_choice 不受支持"),
            }
        } else if choice["type"] == "function" {
            json!({"type":"function", "function":{"name":string(choice, "name")?}})
        } else {
            return Err("tool_choice 仅支持 auto、none、required 或单个函数");
        };
        body.insert("tool_choice".into(), choice);
    }
    if let Some(text) = root.get("text").filter(|v| !v.is_null()) {
        if !text.is_object() {
            return Err("text 须为对象");
        }
        if let Some(format) = text.get("format") {
            let converted = match string(format, "type")? {
                "text" => None,
                "json_object" => Some(json!({"type":"json_object"})),
                "json_schema" => {
                    let mut schema = format.as_object().ok_or("JSON schema 格式不正确")?.clone();
                    schema.remove("type");
                    Some(json!({"type":"json_schema", "json_schema":schema}))
                }
                _ => return Err("text.format 不受支持"),
            };
            if let Some(format) = converted {
                body.insert("response_format".into(), format);
            }
        }
    }
    Ok((body, stream))
}
