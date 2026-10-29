//! 插件管理路由注册与来源、用量、定价操作的类型化入口

use crate::{
    accounts,
    api::{self, ApiError, Result},
    history_pricing, host, key_models, model_priority,
    model_ua::{self, Configuration, RequestLogs},
    pricing,
    sources::SourceStore,
    state, upstream, upstream_pin, usage,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use gateway_plugin_sdk::{
    PluginFault,
    call::{
        host::{KeyListRequest, ModelListRequest, ModelListResult},
        management::{
            ManagementPage, ManagementRegistration, ManagementRequest, ManagementResource,
            ManagementResponse, ManagementRoute,
        },
        middleware::MiddlewareHeader,
        services::settings::SyncPricing,
    },
    client::{TypedCall, TypedReply},
};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn registration() -> ManagementRegistration {
    let get = [
        "api/keys",
        "api/ua",
        "api/pricing",
        "api/usage/adjustment",
        "api/upstream/accounts",
        "api/upstream/support",
        "api/model-priority",
        "api/upstream-pin",
    ];
    let post = [
        "api/key-models/get",
        "api/key-models/save",
        "api/upstream-pin/save",
        "api/upstream-pin/delete",
        "api/upstream-pin/logs",
        "api/upstream-pin/logs/clear",
        "api/models",
        "api/model-priority/lookup",
        "api/model-priority/save",
        "api/model-priority/delete",
        "api/model-priority/logs",
        "api/model-priority/logs/clear",
        "api/upstream/get",
        "api/upstream/save",
        "api/upstream/refresh",
        "api/upstream/sharing",
        "api/upstream/sharing/save",
        "api/ua/save",
        "api/ua/preview",
        "api/ua/diagnose",
        "api/ua/logs",
        "api/ua/logs/clear",
        "api/pricing/update",
        "api/pricing/preview-sync",
        "api/pricing/sync",
        "api/pricing/community",
        "api/usage/summary",
        "api/uploads/start",
        "api/uploads/chunk",
        "api/uploads/finish",
        "api/uploads/cancel",
        "api/import/preview",
        "api/import/key",
        "api/import/save",
        "api/import/usage",
        "api/import/usage/prepare",
        "api/import/usage/apply",
        "api/import/usage/discard",
        "api/usage/adjust",
        "api/pricing/history",
    ];
    let mut routes = Vec::new();
    for (method, paths) in [("GET", get.as_slice()), ("POST", post.as_slice())] {
        for path in paths {
            routes.push(ManagementRoute {
                method: method.into(),
                path: (*path).into(),
                request_content_types: if method == "POST" {
                    vec!["application/json".into()]
                } else {
                    vec![]
                },
                response_content_types: vec!["application/json".into()],
            });
        }
    }
    ManagementRegistration {
        routes,
        resources: ["web/index.html", "web/app.js", "web/app.css"]
            .into_iter()
            .map(|path| ManagementResource {
                path: path.into(),
                public: false,
            })
            .collect(),
        pages: vec![ManagementPage {
            id: "toolkit".into(),
            title: "CPR 数据工具箱".into(),
            description: Some(
                "API密钥导入、CC-Switch用量导入、用量调整、模型批量定价、模型UA、上游额度、模型优先级、钉上游与下游模型权限".into(),
            ),
            entry: "web/index.html".into(),
            icon: None,
        }],
        callbacks: vec![],
    }
}
pub async fn handle(
    sources: &SourceStore,
    logs: &RequestLogs,
    imports: &usage::Imports,
    priority_logs: &model_priority::logs::Logs,
    pin_logs: &upstream_pin::Logs,
    call: TypedCall<ManagementRequest>,
) -> std::result::Result<TypedReply<ManagementResponse>, PluginFault> {
    match route(sources, logs, imports, priority_logs, pin_logs, call).await {
        Ok(value) => api::reply(200, &value),
        Err(error) => error.reply(),
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceId {
    source_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Models {
    client_key_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SaveRules {
    expected_version: Option<u64>,
    value: Configuration,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewRules {
    model: String,
    value: Configuration,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Diagnose {
    client_key_id: String,
    model: String,
    baseline_ua: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UploadStart {
    size: usize,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Chunk {
    source_id: String,
    offset: usize,
    data: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Import {
    source_id: String,
    selections: Vec<accounts::Selection>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ImportKey {
    source_id: String,
    candidate_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Summary {
    period: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Community {
    provider: String,
    offset: usize,
    #[serde(default)]
    search: String,
    #[serde(default)]
    model_filter: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogQuery {
    offset: usize,
}

async fn route(
    sources: &SourceStore,
    logs: &RequestLogs,
    imports: &usage::Imports,
    priority_logs: &model_priority::logs::Logs,
    pin_logs: &upstream_pin::Logs,
    call: TypedCall<ManagementRequest>,
) -> Result<Value> {
    if !call.request.query.is_empty()
        || call.payload.len() > 512 * 1024
        || (call.request.method == "GET" && !call.payload.is_empty())
    {
        return Err(ApiError::invalid("请求查询参数或正文不符合管理接口约定"));
    }
    let host = &call.host;
    let request_id = call
        .context
        .request_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    match (call.request.method.as_str(), call.request.path.as_str()) {
        ("POST", "api/key-models/get") => key_models::get(host, api::decode(&call.payload)?).await,
        ("POST", "api/key-models/save") => key_models::save(host, api::decode(&call.payload)?).await,
        ("GET", "api/upstream-pin") => upstream_pin::get(host).await,
        ("POST", "api/upstream-pin/save") => {
            upstream_pin::save(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/upstream-pin/delete") => {
            upstream_pin::delete(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/upstream-pin/logs") => {
            let request: LogQuery = api::decode(&call.payload)?;
            pin_logs.page(request.offset)
        }
        ("POST", "api/upstream-pin/logs/clear") => {
            require_empty_object(&call.payload)?;
            pin_logs.clear();
            Ok(json!({"cleared":true}))
        }
        ("POST", "api/model-priority/logs") => {
            let request: LogQuery = api::decode(&call.payload)?;
            Ok(serde_json::to_value(
                priority_logs.page(request.offset).await?,
            )?)
        }
        ("POST", "api/model-priority/logs/clear") => {
            require_empty_object(&call.payload)?;
            priority_logs.clear().await;
            Ok(json!({"cleared":true}))
        }
        ("GET", "api/model-priority") => model_priority::get(host).await,
        ("POST", "api/model-priority/lookup") => {
            model_priority::lookup(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/model-priority/save") => {
            model_priority::save(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/model-priority/delete") => {
            model_priority::delete(host, api::decode(&call.payload)?).await
        }
        ("GET", "api/upstream/support") => {
            Ok(serde_json::to_value(upstream::support::get(host).await?)?)
        }
        ("GET", "api/upstream/accounts") => upstream::accounts(host).await,
        ("POST", "api/upstream/get") => upstream::get(host, api::decode(&call.payload)?).await,
        ("POST", "api/upstream/save") => upstream::save(host, api::decode(&call.payload)?).await,
        ("POST", "api/upstream/refresh") => {
            upstream::refresh(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/upstream/sharing") => {
            upstream::sharing(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/upstream/sharing/save") => {
            upstream::save_sharing(host, api::decode(&call.payload)?).await
        }
        ("GET", "api/keys") => {
            let mut keys = Vec::new();
            let mut cursor = None;
            loop {
                let page = host
                    .list_keys(KeyListRequest { cursor, limit: 200 })
                    .await?;
                keys.extend(page.keys);
                if keys.len() > 5000 {
                    return Err(ApiError::invalid("API密钥目录超过当前选择器容量"));
                }
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok(json!({"keys":keys}))
        }
        ("POST", "api/models") => {
            let request: Models = api::decode(&call.payload)?;
            let models: ModelListResult = host::metadata(
                host,
                "host.models.list",
                &ModelListRequest {
                    client_key_id: request.client_key_id,
                    protocol: "openai".into(),
                    client_version: concat!("cpr-data-toolkit/", env!("CARGO_PKG_VERSION")).into(),
                },
            )
            .await?;
            Ok(serde_json::to_value(models)?)
        }
        ("GET", "api/ua") => {
            let (version, value) = model_ua::load(host).await?;
            Ok(json!({"version":version,"value":value}))
        }
        ("POST", "api/ua/logs") => {
            let request: LogQuery = api::decode(&call.payload)?;
            Ok(serde_json::to_value(logs.page(request.offset).await?)?)
        }
        ("POST", "api/ua/logs/clear") => {
            require_empty_object(&call.payload)?;
            logs.clear().await;
            Ok(json!({"cleared":true}))
        }
        ("POST", "api/ua/save") => {
            let request: SaveRules = api::decode(&call.payload)?;
            request.value.validate()?;
            let version = state::put(
                host,
                "model_ua",
                "rules",
                request.expected_version,
                &request.value,
            )
            .await?;
            Ok(json!({"version":version}))
        }
        ("POST", "api/ua/preview") => {
            let request: PreviewRules = api::decode(&call.payload)?;
            request.value.validate()?;
            if request.model.len() > 256 {
                return Err(ApiError::invalid("模型名过长"));
            }
            let mut headers = Vec::new();
            let rule = request.value.rewrite(Some(&request.model), &mut headers);
            Ok(json!({"ruleId":rule.map(|r|&r.id),"headers":headers_view(&headers)}))
        }
        ("POST", "api/ua/diagnose") => {
            let request: Diagnose = api::decode(&call.payload)?;
            let models: ModelListResult = host::metadata(
                host,
                "host.models.list",
                &ModelListRequest {
                    client_key_id: request.client_key_id.clone(),
                    protocol: "openai".into(),
                    client_version: concat!("cpr-data-toolkit/", env!("CARGO_PKG_VERSION")).into(),
                },
            )
            .await?;
            if !models.models.contains(&request.model) {
                return Err(ApiError::invalid("模型不在该API密钥当前可见目录中"));
            }
            let (_, config) = model_ua::load(host).await?;
            let mut headers = Vec::new();
            if let Some(value) = &request.baseline_ua {
                let check = Configuration {
                    rules: vec![model_ua::Rule {
                        id: "baseline".into(),
                        enabled: true,
                        keyword: "baseline".into(),
                        user_agent: value.clone(),
                        originator: None,
                        version: None,
                    }],
                };
                check.validate()?;
                headers.push(MiddlewareHeader {
                    name: "user-agent".into(),
                    value: value.as_bytes().to_vec(),
                });
            }
            let rule = config.rewrite(Some(&request.model), &mut headers);
            Ok(
                json!({"endpoint":"/v1/responses","clientKeyId":request.client_key_id,"model":request.model,"ruleId":rule.map(|r|&r.id),"headers":headers_view(&headers),"baselineKnown":request.baseline_ua.is_some(),"outboundSent":false,"coverage":"shared_request_header_rewrite","stages":["模拟 Responses 请求","查询API密钥可见模型","应用已保存的正式规则","检查插件请求出口"],"notCovered":["真实入口鉴权","宿主绑定筛选","账号选择","默认画像组装","最终网络传输"],"message":"验证缩短至插件请求头改写，未调用真实 /v1/responses，也未发起上游推理请求"}),
            )
        }
        ("GET", "api/pricing") => pricing::catalog(host).await,
        ("GET", "api/usage/adjustment") => usage::pending_adjustment(host).await,
        ("POST", "api/usage/adjust") => {
            let _guard = sources.import_lock.lock().await;
            usage::adjust(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/import/usage/prepare") => {
            let options: usage::ImportOptions = api::decode(&call.payload)?;
            let source = sources.get(&options.source_id).await?;
            imports.prepare(host, &source, options).await
        }
        ("POST", "api/import/usage/apply") => {
            imports.apply(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/import/usage/discard") => imports.discard(api::decode(&call.payload)?).await,
        ("POST", "api/pricing/history") => {
            let _guard = sources.import_lock.lock().await;
            history_pricing::sync_page(host, api::decode(&call.payload)?).await
        }
        ("POST", "api/pricing/update") => {
            pricing::update(host, request_id, api::decode(&call.payload)?).await
        }
        ("POST", "api/pricing/preview-sync") => {
            require_empty_object(&call.payload)?;
            pricing::preview_sync(host).await
        }
        ("POST", "api/pricing/sync") => {
            let command: SyncPricing = api::decode(&call.payload)?;
            pricing::sync(host, request_id, command).await
        }
        ("POST", "api/pricing/community") => {
            let request: Community = api::decode(&call.payload)?;
            pricing::community(
                host,
                &request.provider,
                request.offset,
                &request.search,
                &request.model_filter,
            )
            .await
        }
        ("POST", "api/usage/summary") => {
            let request: Summary = api::decode(&call.payload)?;
            if !matches!(request.period.as_str(), "today" | "7d" | "30d") {
                return Err(ApiError::invalid("统计范围无效"));
            }
            let (tokens, requests) =
                usage::read_summary(host, &format!("period={}", request.period)).await?;
            Ok(
                json!({"period":request.period,"tokens":tokens.to_string(),"requests":requests.to_string()}),
            )
        }
        ("POST", "api/uploads/start") => {
            let request: UploadStart = api::decode(&call.payload)?;
            let id = sources.start(request.size).await?;
            Ok(json!({"sourceId":id}))
        }
        ("POST", "api/uploads/chunk") => {
            let request: Chunk = api::decode(&call.payload)?;
            if request.data.len() > 350000 {
                return Err(ApiError::invalid("上传分块超过容量"));
            }
            let bytes = STANDARD
                .decode(request.data)
                .map_err(|_| ApiError::invalid("上传分块编码无效"))?;
            Ok(json!({"offset":sources.chunk(&request.source_id,request.offset,&bytes).await?}))
        }
        ("POST", "api/uploads/finish") => {
            let request: SourceId = api::decode(&call.payload)?;
            let source = sources.finish(&request.source_id).await?;
            Ok(
                json!({"sourceId":request.source_id,"providers":source.candidates.len(),"usageGroups":source.usage.groups.len()}),
            )
        }
        ("POST", "api/uploads/cancel") => {
            let request: SourceId = api::decode(&call.payload)?;
            sources.cancel(&request.source_id).await;
            Ok(json!({"cleared":true}))
        }
        ("POST", "api/import/preview") => {
            let request: SourceId = api::decode(&call.payload)?;
            let source = sources.get(&request.source_id).await?;
            accounts::preview(host, &source).await
        }
        ("POST", "api/import/key") => {
            let request: ImportKey = api::decode(&call.payload)?;
            let source = sources.get(&request.source_id).await?;
            accounts::key(&source, &request.candidate_id)
        }
        ("POST", "api/import/save") => {
            let request: Import = api::decode(&call.payload)?;
            let _guard = sources.import_lock.lock().await;
            let source = sources.get(&request.source_id).await?;
            accounts::import(host, &source, request.selections).await
        }
        ("POST", "api/import/usage") => {
            let request: usage::PreviewPage = api::decode(&call.payload)?;
            let source = if request.task_id.is_none() {
                Some(sources.get(&request.options.source_id).await?)
            } else {
                None
            };
            imports.preview(source.as_deref(), request).await
        }
        _ => Err(ApiError::new(404, "not_found", "未找到插件接口")),
    }
}
fn headers_view(headers: &[MiddlewareHeader]) -> Vec<Value> {
    headers
        .iter()
        .map(|h| json!({"name":h.name,"value":String::from_utf8_lossy(&h.value)}))
        .collect()
}
fn require_empty_object(bytes: &[u8]) -> Result<()> {
    if api::decode::<Value>(bytes)? != json!({}) {
        Err(ApiError::invalid("此操作不接受附加参数"))
    } else {
        Ok(())
    }
}
