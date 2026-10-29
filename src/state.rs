use crate::host;
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{
        StateGetRequest, StateGetResult, StateMigrationChange, StateMigrationRequest,
        StateMigrationResult, StatePutRequest, StatePutResult,
    },
    client::HostClient,
};
use serde::{Serialize, de::DeserializeOwned};

const UPSTREAM_QUOTA_SCHEMA_VERSION: u32 = 2;

pub fn migrate(request: StateMigrationRequest) -> Result<StateMigrationResult, PluginFault> {
    if request.namespace != "upstream_quota"
        || request.from_schema_version != 1
        || request.to_schema_version != UPSTREAM_QUOTA_SCHEMA_VERSION
    {
        return Err(PluginFault::new(
            ErrorCode::InvalidInput,
            "不支持的插件状态迁移路径",
        ));
    }
    // 新 schema 只扩充查询类型；逐项 Keep 保留原键和值，记录版本由宿主的新代次管理
    Ok(StateMigrationResult {
        changes: request
            .records
            .into_iter()
            .map(|record| StateMigrationChange::Keep { key: record.key })
            .collect(),
    })
}

pub async fn get<T: DeserializeOwned>(
    host: &HostClient,
    namespace: &str,
    key: &str,
) -> Result<(Option<u64>, Option<T>), PluginFault> {
    let result: StateGetResult = host::metadata(
        host,
        "host.state.get",
        &StateGetRequest {
            namespace: namespace.into(),
            key: key.into(),
        },
    )
    .await?;
    match result.record {
        None => Ok((None, None)),
        Some(record) => {
            let expected_schema_version = if namespace == "upstream_quota" {
                UPSTREAM_QUOTA_SCHEMA_VERSION
            } else {
                1
            };
            if record.schema_version != expected_schema_version {
                return Err(PluginFault::new(ErrorCode::Fault, "状态版本不受支持"));
            }
            let value = serde_json::from_value(record.value)
                .map_err(|_| PluginFault::new(ErrorCode::Fault, "状态数据无效"))?;
            Ok((Some(record.version), Some(value)))
        }
    }
}
pub async fn put<T: Serialize>(
    host: &HostClient,
    namespace: &str,
    key: &str,
    version: Option<u64>,
    value: &T,
) -> Result<u64, PluginFault> {
    let value = serde_json::to_value(value)
        .map_err(|_| PluginFault::new(ErrorCode::Fault, "状态编码失败"))?;
    let result: StatePutResult = host::metadata(
        host,
        "host.state.put",
        &StatePutRequest {
            namespace: namespace.into(),
            key: key.into(),
            expected_version: version,
            value,
        },
    )
    .await?;
    Ok(result.version)
}
