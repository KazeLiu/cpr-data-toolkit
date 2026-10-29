//! API密钥的下游模型白名单；只收紧宿主已经授予的权限

use crate::{
    api::{ApiError, Result},
    model_ua::{self, RequestLogs},
    state,
};
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        host::KeyListRequest,
        middleware::{MiddlewareBodyFrame, MiddlewareBodyFraming, MiddlewareHeader},
    },
    client::{HostClient, MiddlewareBody, MiddlewareResponse, RequestCall},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

const NAMESPACE: &str = "key_models";
const MAX_MODELS: usize = 1000;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    enabled: bool,
    models: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    client_key_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    client_key_id: String,
    expected_version: Option<u64>,
    value: Policy,
}

async fn ensure_key(host: &HostClient, key: &str) -> Result<()> {
    if key.is_empty() || key.len() > 256 {
        return Err(ApiError::invalid("请选择API密钥"));
    }
    let mut cursor = None;
    let mut count = 0;
    loop {
        let page = host.list_keys(KeyListRequest { cursor, limit: 200 }).await?;
        if page.keys.iter().any(|entry| entry.id == key) {
            return Ok(());
        }
        count += page.keys.len();
        if count > 5000 {
            return Err(ApiError::invalid("API密钥目录超过当前选择器容量"));
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => return Err(ApiError::new(404, "key_not_found", "API密钥已删除，请重新选择")),
        }
    }
}

async fn load(
    host: &HostClient,
    key: &str,
) -> std::result::Result<(Option<u64>, Policy), PluginFault> {
    let (version, value) = state::get(host, NAMESPACE, key).await?;
    Ok((version, value.unwrap_or_default()))
}

pub async fn allowed_models(
    host: &HostClient,
    key: &str,
) -> std::result::Result<Option<HashSet<String>>, PluginFault> {
    let (_, policy) = load(host, key).await?;
    Ok(policy.enabled.then(|| policy.models.into_iter().collect()))
}

pub async fn get(host: &HostClient, selection: Selection) -> Result<Value> {
    ensure_key(host, &selection.client_key_id).await?;
    let (version, value) = load(host, &selection.client_key_id).await?;
    Ok(json!({"version": version, "value": value}))
}

pub async fn save(host: &HostClient, request: Save) -> Result<Value> {
    ensure_key(host, &request.client_key_id).await?;
    if request.value.models.len() > MAX_MODELS {
        return Err(ApiError::invalid("每个API密钥最多允许 1000 个模型"));
    }
    let mut seen = HashSet::new();
    for model in &request.value.models {
        if model.is_empty()
            || model.len() > 512
            || model.trim() != model
            || model.chars().any(char::is_control)
            || !seen.insert(model)
        {
            return Err(ApiError::invalid(
                "模型 ID 不能为空、重复或含控制字符，最多 512 字节",
            ));
        }
    }
    if serde_json::to_vec(&request.value)?.len() > 262144 {
        return Err(ApiError::invalid("模型白名单超过容量限制"));
    }
    let version = state::put(
        host,
        NAMESPACE,
        &request.client_key_id,
        request.expected_version,
        &request.value,
    )
    .await?;
    Ok(json!({"version": version, "value": request.value}))
}

pub async fn middleware(
    logs: &RequestLogs,
    call: RequestCall,
) -> std::result::Result<MiddlewareResponse, PluginFault> {
    let catalog = call.request.head.endpoint == "/v1/models";
    // 用量等无模型的只读查询不属于模型权限范围
    if !catalog && call.request.head.operation == "query" && call.request.head.model.is_none() {
        return model_ua::middleware(logs, call).await;
    }
    let allowed = match allowed_models(&call.host, &call.request.head.client_key_id).await {
        Ok(allowed) => allowed,
        Err(_) => {
            // 返回明确响应，避免宿主的 delegate 失败策略跳过权限检查
            return error_response(
                &call.request.head.protocol,
                503,
                "model_policy_unavailable",
                "下游模型权限读取失败，请稍后重试",
            );
        }
    };
    let Some(allowed) = allowed else {
        return model_ua::middleware(logs, call).await;
    };
    // 图片等原生 JSON 端点尚未提供 head.model，按正文中的公开 model 检查
    let model = call.request.head.model.clone().or_else(|| {
        serde_json::from_slice::<Value>(&call.request.body)
            .ok()?
            .get("model")?
            .as_str()
            .map(str::to_owned)
    });
    if !catalog && !model.as_ref().is_some_and(|model| allowed.contains(model)) {
        return error_response(
            &call.request.head.protocol,
            403,
            "model_not_allowed",
            if model.is_some() {
                "此API密钥未获准请求该模型，请联系管理员调整模型白名单"
            } else {
                "此API密钥已限制模型，无法确认请求模型，请使用明确提供 model 的 JSON 请求"
            },
        );
    }
    let protocol = call.request.head.protocol.clone();
    let mut response = match model_ua::middleware(logs, call).await {
        Ok(response) => response,
        Err(error) if !catalog => return Err(error),
        Err(_) => {
            return error_response(
                &protocol,
                503,
                "model_catalog_unavailable",
                "模型目录暂不可用，请稍后重试",
            );
        }
    };
    if catalog && (200..300).contains(&response.status) {
        if response.body.framing() != Some(MiddlewareBodyFraming::JsonDocument) {
            response.body.close().await?;
            return Err(invalid_catalog());
        }
        response.body = response.body.map_frames(move |mut frame| {
            let mut value: Value =
                serde_json::from_slice(&frame.payload).map_err(|_| invalid_catalog())?;
            let (field, id) = if value.get("data").is_some() {
                ("data", "id")
            } else {
                ("models", "slug")
            };
            let models = value
                .get_mut(field)
                .and_then(Value::as_array_mut)
                .ok_or_else(invalid_catalog)?;
            if models
                .iter()
                .any(|model| model.get(id).and_then(Value::as_str).is_none())
            {
                return Err(invalid_catalog());
            }
            models.retain(|model| {
                allowed.contains(model[id].as_str().expect("catalog IDs checked above"))
            });
            frame.payload = serde_json::to_vec(&value).map_err(|_| invalid_catalog())?;
            Ok(vec![frame])
        })?;
        for name in ["content-length", "etag", "content-md5", "cache-control"] {
            response.remove_header(name);
        }
        response.append_header("cache-control", b"private, no-store".to_vec());
    }
    Ok(response)
}

fn header(name: &str, value: &str) -> MiddlewareHeader {
    MiddlewareHeader {
        name: name.into(),
        value: value.as_bytes().to_vec(),
    }
}

fn error_response(
    protocol: &str,
    status: u16,
    code: &str,
    message: &str,
) -> std::result::Result<MiddlewareResponse, PluginFault> {
    let body = serde_json::to_vec(&json!({"error": {
        "message": message,
        "type": if status == 403 { "permission_error" } else { "server_error" },
        "code": code,
    }}))
    .map_err(|_| PluginFault::new(ErrorCode::Fault, "模型权限响应编码失败"))?;
    Ok(MiddlewareResponse::direct(
        protocol,
        status,
        vec![
            header("content-type", "application/json"),
            header("cache-control", "no-store"),
        ],
        MiddlewareBody::from_frames(
            MiddlewareBodyFraming::JsonDocument,
            vec![MiddlewareBodyFrame::new(body, true)],
        ),
    ))
}

fn invalid_catalog() -> PluginFault {
    PluginFault::new(
        ErrorCode::Fault,
        "模型目录格式不兼容，无法安全应用 下游模型权限",
    )
}
