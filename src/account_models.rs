//! 通过公开宿主 HTTP 合同读取指定账号的模型目录，不使用整站模型列表

use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    call::middleware::http::Version,
    client::{HostClient, HttpBody, HttpFrame, HttpRequest},
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Envelope {
    data: Models,
}

#[derive(Deserialize)]
struct Models {
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    id: String,
}

pub async fn list(host: &HostClient, account_id: &str) -> Result<Vec<String>> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("accountId", account_id)
        .finish();
    let mut response = host
        .dispatch_http(HttpRequest {
            settings: Value::Null,
            method: "GET".into(),
            uri: format!("/api/admin/accounts/models?{query}"),
            version: Version::Http11,
            headers: vec![],
            timeout_ms: Some(5000),
            body: HttpBody::empty(),
        })
        .await?;
    if response.status != 200 {
        response.body.close().await?;
        return Err(ApiError::new(502, "model_catalog", "模型目录读取失败"));
    }
    let mut bytes = Vec::new();
    while let Some(frame) = response.body.read().await? {
        if let HttpFrame::Data(data) = frame {
            if bytes.len() + data.len() > 1024 * 1024 {
                response.body.close().await?;
                return Err(ApiError::new(502, "model_catalog", "模型目录超过 1 MiB"));
            }
            bytes.extend(data);
        }
    }
    let result: Envelope = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::new(502, "model_catalog", "模型目录格式不兼容"))?;
    Ok(result
        .data
        .models
        .into_iter()
        .map(|model| model.id)
        .collect())
}
