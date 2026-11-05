//! 一次父调用内完整消费宿主聚合快照，不跨调用保存读取句柄

use super::report::DataSet;
use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    call::middleware::http::Version,
    call::services::usage::*,
    client::{HostClient, HttpBody, HttpFrame, HttpRequest},
};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Notify;

#[derive(Clone, Deserialize)]
pub(super) struct Group {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Deserialize)]
pub(super) struct Key {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct KeyPage {
    pub items: Vec<Key>,
    pub next_cursor: Option<String>,
}

pub(super) async fn statistics(
    host: &HostClient,
    filter: UsageRecordFilter,
    group_by: UsageGrouping,
    routing_group_ref: Option<String>,
    cancel: &Notify,
) -> Result<DataSet> {
    let mut reader = tokio::select! {
        _=cancel.notified()=>return Err(ApiError::conflict("统计查询已取消")),
        result=host.query_usage_statistics(QueryUsageStatistics{filter:UsageReadFilter{records:filter,routing_group_ref},group_by,batch_size:Some(256)})=>result.map_err(read_error)?,
    };
    let result = async {
        let mut data = DataSet::default();
        let mut complete = false;
        loop {
            let batch = tokio::select! {
                _=cancel.notified()=>return Err(ApiError::conflict("统计查询已取消")),
                result=reader.next_batch()=>result.map_err(read_error)?,
            };
            let Some(batch) = batch else {
                break;
            };
            if data.observation.is_none() {
                data.observation = Some(batch.observation);
            }
            for item in batch.items {
                match item {
                    UsageReadItem::Statistics(row) => data.push(*row)?,
                    _ => {
                        return Err(ApiError::new(
                            502,
                            "statistics_read",
                            "宿主返回了非聚合结果",
                        ));
                    }
                }
            }
            if batch.complete {
                complete = true;
                break;
            }
        }
        if !complete || data.summary.is_none() {
            return Err(ApiError::new(
                502,
                "statistics_incomplete",
                "聚合查询未完整结束，请继续重查当前维度",
            ));
        }
        Ok(data)
    }
    .await;
    let closed = reader.close().await.map_err(read_error);
    let data = result?;
    closed?;
    Ok(data)
}
fn read_error(error: gateway_plugin_sdk::PluginFault) -> ApiError {
    use gateway_plugin_sdk::ErrorCode;
    match error.code {
        ErrorCode::Capacity => ApiError::invalid("聚合响应超过容量，请缩小范围"),
        ErrorCode::Timeout => ApiError::new(
            504,
            "statistics_timeout",
            "宿主聚合查询超时，可继续重查当前维度或缩小范围",
        ),
        ErrorCode::Cancelled => ApiError::conflict("宿主聚合查询已取消，可继续重查当前维度"),
        _ => ApiError::new(
            502,
            "statistics_read",
            "宿主聚合尚未就绪或读取失败，请检查投影状态后继续重查",
        ),
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
    get_json(host, format!("/api/admin/client-keys?{query}")).await
}

#[derive(Deserialize)]
pub(super) struct GroupPage {
    pub items: Vec<Group>,
    pub page: GroupPages,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GroupPages {
    pub total_pages: usize,
}
pub(super) async fn groups(host: &HostClient, page: usize) -> Result<GroupPage> {
    get_json(
        host,
        format!("/api/admin/account-groups/options?page={page}&pageSize=200"),
    )
    .await
}
async fn get_json<T: DeserializeOwned>(host: &HostClient, uri: String) -> Result<T> {
    let mut response = host
        .dispatch_http(HttpRequest {
            settings: Value::Null,
            method: "GET".into(),
            uri,
            version: Version::Http11,
            headers: vec![],
            timeout_ms: Some(30000),
            body: HttpBody::empty(),
        })
        .await?;
    let result = async {
        if response.status != 200 {
            return Err(ApiError::new(502, "statistics_keys", "名称目录读取失败"));
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
        struct Envelope<T> {
            data: T,
        }
        Ok(serde_json::from_slice::<Envelope<T>>(&bytes)?.data)
    }
    .await;
    response.body.close().await?;
    result
}
