//! 发布 ClinePass 订阅目录；用冻结的账号原目录和 CPR 当前模型映射过滤旧条目

use super::{ADAPTER, InstanceConfiguration};
use crate::{
    account_models,
    api::{ApiError, Result},
};
use gateway_plugin_sdk::{
    PluginFault,
    call::{
        catalog::{AdaptedModel, ModelCatalogRegistrationV2, ModelPresentation},
        data::AccessQuery,
        host::HttpRequest,
        middleware::RequestFeature,
    },
    client::HostClient,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

const MAX_MODELS: usize = 64;
const MAX_ORIGINAL_MODELS: usize = 2000;
const MAX_CONFIGURATION_BYTES: usize = 48 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Replacement {
    model_ids: Vec<String>,
    original_models: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Prepare {
    account_ids: Vec<String>,
}

fn valid_id(id: &str, max: usize) -> bool {
    !id.is_empty() && id.len() <= max && !id.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn pass_id(id: &str) -> bool {
    valid_id(id, 256)
        && id
            .strip_prefix("cline-pass/")
            .is_some_and(|slug| !slug.is_empty())
}

fn unique(values: &[String]) -> bool {
    values.iter().collect::<HashSet<_>>().len() == values.len()
}

async fn list(host: &HostClient) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Catalog {
        #[serde(rename = "clinePass")]
        cline_pass: Vec<Model>,
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }

    let mut response = host
        .http(
            HttpRequest {
                method: "GET".into(),
                url: "https://api.cline.bot/api/v1/ai/cline/recommended-models".into(),
                headers: vec![],
            },
            vec![],
        )
        .await?;
    if response.status != 200 {
        response.body.close().await?;
        return Err(ApiError::new(
            502,
            "cline_catalog",
            "ClinePass 模型目录暂不可用",
        ));
    }
    let bytes = response.body.collect(128 * 1024).await?;
    let catalog: Catalog = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::new(502, "cline_catalog", "ClinePass 模型目录格式无效"))?;
    let mut seen = HashSet::new();
    Ok(catalog
        .cline_pass
        .into_iter()
        .map(|model| model.id)
        .filter(|id| pass_id(id) && seen.insert(id.clone()))
        .collect())
}

pub async fn models(host: &HostClient) -> Result<Value> {
    Ok(json!({"models": list(host).await?}))
}

pub async fn prepare(host: &HostClient, input: Prepare) -> Result<Value> {
    if input.account_ids.is_empty()
        || input.account_ids.len() > 100
        || !unique(&input.account_ids)
        || input.account_ids.iter().any(|id| !valid_id(id, 256))
    {
        return Err(ApiError::invalid("请先选择要替换目录的转换账号"));
    }
    let mut model_ids = list(host).await?;
    if model_ids.is_empty() || model_ids.len() > MAX_MODELS {
        return Err(ApiError::new(
            502,
            "cline_catalog",
            "ClinePass 官方目录为空或超过 64 个模型，无法生成替换配置",
        ));
    }
    let mut original_models = BTreeMap::new();
    for account in &input.account_ids {
        let mut models = account_models::list(host, account).await?;
        models.sort();
        models.dedup();
        if models.len() > MAX_ORIGINAL_MODELS || models.iter().any(|id| !valid_id(id, 512)) {
            return Err(ApiError::invalid("转换账号的原模型目录超出配置容量或格式无效"));
        }
        original_models.insert(account.clone(), models);
    }
    model_ids.sort();
    let config = InstanceConfiguration {
        cline_account_ids: input.account_ids,
        cline_catalog: Some(Replacement {
            model_ids,
            original_models,
        }),
    };
    let config = serde_json::to_value(config)?;
    if serde_json::to_vec(&config)?.len() > MAX_CONFIGURATION_BYTES {
        return Err(ApiError::invalid("模型列表替换配置超过 48 KiB，请减少转换账号"));
    }
    Ok(config)
}

impl Replacement {
    pub(super) fn validate(&self, accounts: &[String]) -> std::result::Result<(), &'static str> {
        if self.model_ids.is_empty()
            || self.model_ids.len() > MAX_MODELS
            || !unique(&self.model_ids)
            || self.model_ids.iter().any(|id| !pass_id(id))
            || self.original_models.len() != accounts.len()
            || accounts.is_empty()
            || accounts.iter().any(|id| !self.original_models.contains_key(id))
            || self.original_models.values().any(|models| {
                models.len() > MAX_ORIGINAL_MODELS
                    || models.iter().any(|id| !valid_id(id, 512))
            })
        {
            return Err("模型列表替换配置无效，请重新复制实例配置");
        }
        Ok(())
    }

    pub(super) fn model_ids(&self) -> &[String] {
        &self.model_ids
    }

    pub(super) fn registration(&self) -> ModelCatalogRegistrationV2 {
        ModelCatalogRegistrationV2 {
            aliases: vec![],
            models: self
                .model_ids
                .iter()
                .map(|id| {
                    // Cline 客户端的订阅目录默认能力是函数工具和推理；
                    // 1017 的目录还明确声明图像输入与结构化输出
                    let is_1017 = id == "cline-pass/deepseek-v4.1-flash-1017";
                    let mut features = vec![RequestFeature::Tools, RequestFeature::Reasoning];
                    if is_1017 {
                        features.extend([RequestFeature::Vision, RequestFeature::JsonSchema]);
                    }
                    AdaptedModel {
                        id: id.clone(),
                        adapter_id: ADAPTER.into(),
                        features,
                        max_output_tokens: None,
                        presentation: ModelPresentation {
                            display_name: Some(id.clone()),
                            image_input: is_1017,
                            agent_tools: true,
                            ..Default::default()
                        },
                    }
                })
                .collect(),
        }
    }

    pub(super) async fn hidden_models(
        &self,
        host: &HostClient,
        key: &str,
        settings: &Value,
    ) -> std::result::Result<HashSet<String>, PluginFault> {
        let access = host
            .check_access(AccessQuery {
                client_key_id: key.into(),
                account_ids: self.original_models.keys().cloned().collect(),
                upstream_model: None,
            })
            .await?;
        let mut hidden: HashSet<_> = access
            .accounts
            .into_iter()
            .filter(|account| account.allowed)
            .filter_map(|account| self.original_models.get(&account.account_id))
            .flatten()
            .filter(|id| !self.model_ids.contains(id))
            .cloned()
            .collect();
        if !hidden.is_empty()
            && let Some(mappings) = settings.get("model_mappings").and_then(Value::as_object)
        {
            let aliases: Vec<_> = mappings
                .keys()
                .filter(|id| {
                    !self.model_ids.contains(id) && hidden.contains(mapped_model(id, mappings))
                })
                .cloned()
                .collect();
            hidden.extend(aliases);
        }
        Ok(hidden)
    }
}

fn mapped_model<'a>(id: &'a str, mappings: &'a serde_json::Map<String, Value>) -> &'a str {
    let mut current = id;
    // 与 CPR 的模型映射一致：最多追踪 20 次，过深或成环时保留原名
    for _ in 0..20 {
        let Some(target) = mappings.get(current).and_then(Value::as_str) else {
            return current;
        };
        current = target;
    }
    id
}
