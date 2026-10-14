use crate::host;
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::host::{StateGetRequest, StateGetResult, StatePutRequest, StatePutResult},
    client::HostClient,
};
use serde::{Serialize, de::DeserializeOwned};

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
            if record.schema_version != 1 {
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
