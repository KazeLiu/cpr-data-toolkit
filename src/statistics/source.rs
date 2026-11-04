//! 统计读取的公开 SDK 分页与当前 Key 分组目录，宿主数据保持只读

use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    call::middleware::http::Version,
    call::services::usage::*,
    client::{HostClient, HttpBody, HttpFrame, HttpRequest},
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Deserialize)]
pub(super) struct Group {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Deserialize)]
pub(super) struct Key {
    pub id: String,
    pub name: String,
    pub groups: Vec<Group>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyPage {
    pub items: Vec<Key>,
    pub next_cursor: Option<String>,
}

// shortcut: 当前逐页读取历史事实；待 #592 补齐通用聚合接口后，在此模块替换为宿主聚合查询
// https://github.com/zyycn/codex-proxy-rs/issues/592
pub(super) async fn records(
    host: &HostClient,
    filter: UsageRecordFilter,
    cursor: Option<String>,
) -> Result<UsageRecordPage> {
    host.service::<ListRecords>(ListUsageRecords {
        filter,
        cursor,
        limit: Some(50),
    })
    .await
    .map_err(read_error)
}

pub(super) async fn aggregates(
    host: &HostClient,
    filter: UsageRecordFilter,
    cursor: Option<String>,
) -> Result<UsageAggregatePage> {
    host.service::<ListAggregates>(ListUsageAggregates {
        filter,
        cursor,
        limit: Some(100),
    })
    .await
    .map_err(read_error)
}

fn read_error(error: gateway_plugin_sdk::call::services::ServiceError) -> ApiError {
    if error.kind == "capacity_exceeded" {
        ApiError::invalid(
            "宿主单页历史响应超过容量，请缩小范围后重查，持续失败时需等待历史批量读取接口",
        )
    } else {
        ApiError::new(
            502,
            "statistics_read",
            "历史数据读取失败，可继续同一统计任务",
        )
    }
}

pub(super) async fn keys(host: &HostClient, cursor: Option<&str>) -> Result<KeyPage> {
    let query = {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        query.append_pair("limit", "100");
        if let Some(cursor) = cursor {
            query.append_pair("cursor", cursor);
        }
        query.finish()
    };
    let mut response = host
        .dispatch_http(HttpRequest {
            settings: Value::Null,
            method: "GET".into(),
            uri: format!("/api/admin/client-keys?{query}"),
            version: Version::Http11,
            headers: vec![],
            timeout_ms: Some(30000),
            body: HttpBody::empty(),
        })
        .await?;
    let result = async {
        if response.status != 200 {
            return Err(ApiError::new(
                502,
                "statistics_keys",
                "当前 Key 分组目录读取失败",
            ));
        }
        let mut bytes = Vec::new();
        while let Some(frame) = response.body.read().await? {
            if let HttpFrame::Data(data) = frame {
                if bytes.len() + data.len() > 2 * 1024 * 1024 {
                    return Err(ApiError::new(
                        502,
                        "statistics_keys",
                        "Key 目录响应超过容量",
                    ));
                }
                bytes.extend(data);
            }
        }
        #[derive(Deserialize)]
        struct Envelope {
            data: KeyPage,
        }
        Ok(serde_json::from_slice::<Envelope>(&bytes)?.data)
    }
    .await;
    response.body.close().await?;
    result
}
