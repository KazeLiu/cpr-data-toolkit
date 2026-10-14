use crate::api::{ApiError, Result};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_FILE: usize = 64 * 1024 * 1024;
const MAX_CHUNK: usize = 256 * 1024;
const TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
pub struct Candidate {
    pub id: String,
    pub app: String,
    pub provider: String,
    pub name: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub protocol: Option<String>,
    pub model: Option<String>,
    pub endpoints: Vec<String>,
    pub reason: Option<String>,
}
#[derive(Clone)]
pub struct ParsedSource {
    pub candidates: Vec<Candidate>,
    pub usage: UsagePreview,
}
struct Upload {
    size: usize,
    bytes: Vec<u8>,
    touched: Instant,
}
struct Source {
    parsed: ParsedSource,
    touched: Instant,
}
#[derive(Default)]
pub struct SourceStore {
    uploads: Mutex<HashMap<String, Upload>>,
    sources: Mutex<HashMap<String, Source>>,
    pub import_lock: Mutex<()>,
}
impl SourceStore {
    pub async fn expire(&self) {
        self.uploads
            .lock()
            .await
            .retain(|_, value| value.touched.elapsed() < TTL);
        self.sources
            .lock()
            .await
            .retain(|_, value| value.touched.elapsed() < TTL);
    }
    pub async fn start(&self, size: usize) -> Result<String> {
        if size == 0 || size > MAX_FILE {
            return Err(ApiError::invalid("SQL 文件须为 1 字节至 64 MiB"));
        }
        self.expire().await;
        let mut uploads = self.uploads.lock().await;
        if uploads.len() >= 2 {
            return Err(ApiError::new(
                429,
                "capacity",
                "已有两个上传任务，请完成或取消后继续",
            ));
        }
        let id = Uuid::new_v4().to_string();
        uploads.insert(
            id.clone(),
            Upload {
                size,
                bytes: Vec::new(),
                touched: Instant::now(),
            },
        );
        Ok(id)
    }
    pub async fn chunk(&self, id: &str, offset: usize, bytes: &[u8]) -> Result<usize> {
        if bytes.is_empty() || bytes.len() > MAX_CHUNK {
            return Err(ApiError::invalid("上传分块无效"));
        }
        let mut uploads = self.uploads.lock().await;
        let upload = uploads
            .get_mut(id)
            .ok_or_else(|| ApiError::new(404, "expired", "上传任务不存在或已过期"))?;
        if upload.touched.elapsed() >= TTL
            || offset != upload.bytes.len()
            || upload.bytes.len() + bytes.len() > upload.size
        {
            return Err(ApiError::conflict("上传顺序或长度不一致，请重新上传"));
        }
        upload.bytes.extend_from_slice(bytes);
        upload.touched = Instant::now();
        Ok(upload.bytes.len())
    }
    pub async fn finish(&self, id: &str) -> Result<ParsedSource> {
        let upload = self
            .uploads
            .lock()
            .await
            .remove(id)
            .ok_or_else(|| ApiError::new(404, "expired", "上传任务不存在或已过期"))?;
        if upload.touched.elapsed() >= TTL || upload.bytes.len() != upload.size {
            return Err(ApiError::invalid("文件上传尚未完成或已经过期"));
        }
        let parsed = tokio::task::spawn_blocking(move || parse(&upload.bytes))
            .await
            .map_err(|_| ApiError::new(500, "parse", "来源文件处理失败"))??;
        let mut sources = self.sources.lock().await;
        sources.retain(|_, source| source.touched.elapsed() < TTL);
        if sources.len() >= 2 {
            return Err(ApiError::new(
                429,
                "capacity",
                "已有两个来源，请关闭不用的来源后继续",
            ));
        }
        sources.insert(
            id.into(),
            Source {
                parsed: parsed.clone(),
                touched: Instant::now(),
            },
        );
        Ok(parsed)
    }
    pub async fn get(&self, id: &str) -> Result<ParsedSource> {
        let mut sources = self.sources.lock().await;
        let source = sources
            .get_mut(id)
            .ok_or_else(|| ApiError::new(404, "expired", "来源已清理，请重新上传"))?;
        if source.touched.elapsed() >= TTL {
            sources.remove(id);
            return Err(ApiError::new(404, "expired", "来源已过期，请重新上传"));
        }
        source.touched = Instant::now();
        Ok(source.parsed.clone())
    }
    pub async fn cancel(&self, id: &str) {
        self.uploads.lock().await.remove(id);
        self.sources.lock().await.remove(id);
    }
}
fn database_error(error: rusqlite::Error) -> ApiError {
    let message = match error {
        rusqlite::Error::SqliteFailure(code, _)
            if code.code == rusqlite::ErrorCode::AuthorizationForStatementDenied =>
        {
            "SQL 包含不允许的操作，未写入宿主"
        }
        rusqlite::Error::InvalidColumnType(..) => "来源字段类型不符合 schema 合同",
        _ => "SQL 结构或执行内容不受支持，原始内容未写入宿主",
    };
    ApiError::invalid(message)
}
fn authorize(context: AuthContext<'_>) -> Authorization {
    let allowed = match context.action {
        AuthAction::CreateTable { .. }
        | AuthAction::CreateIndex { .. }
        | AuthAction::Insert { .. }
        | AuthAction::Read { .. }
        | AuthAction::Select
        | AuthAction::Transaction { .. } => true,
        // SQLite 创建索引时也会请求 REINDEX 授权，仅允许隔离内存库的索引操作。
        AuthAction::Reindex { .. } => context.database_name == Some("main"),
        AuthAction::Update { table_name, .. } => {
            matches!(table_name, "sqlite_master" | "sqlite_schema")
        }
        AuthAction::Delete { table_name } => table_name == "sqlite_sequence",
        AuthAction::Function { function_name } => matches!(
            function_name.to_ascii_lowercase().as_str(),
            "coalesce" | "lower" | "upper" | "length" | "datetime"
        ),
        AuthAction::Pragma { pragma_name, .. } => matches!(
            pragma_name.to_ascii_lowercase().as_str(),
            "foreign_keys" | "user_version"
        ),
        _ => false,
    };
    if allowed && context.database_name.is_none_or(|db| db == "main") {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
fn columns(conn: &Connection, table: &str, expected: &[&str]) -> Result<()> {
    // 表名仅来自下面固定的结构清单，不拼接外部输入。
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(database_error)?;
    let actual = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(database_error)?
        .collect::<std::result::Result<HashSet<_>, _>>()
        .map_err(database_error)?;
    if expected.iter().any(|column| !actual.contains(*column)) {
        return Err(ApiError::invalid(format!(
            "来源结构不兼容：{table} 缺少必要列"
        )));
    }
    Ok(())
}
fn parse(bytes: &[u8]) -> Result<ParsedSource> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| ApiError::invalid("SQL 文件须使用 UTF-8 编码"))?;
    if !text
        .trim_start_matches('\u{feff}')
        .starts_with("-- CC Switch SQLite 导出")
    {
        return Err(ApiError::invalid("请选择 CCS 完整手动 SQL 导出"));
    }
    let conn = Connection::open_in_memory().map_err(database_error)?;
    conn.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA max_page_count=16384;")
        .map_err(database_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_LENGTH, MAX_FILE as i32)
        .map_err(database_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, MAX_FILE as i32)
        .map_err(database_error)?;
    conn.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 100)
        .map_err(database_error)?;
    let started = Instant::now();
    conn.progress_handler(
        10000,
        Some(move || started.elapsed() > Duration::from_secs(15)),
    )
    .map_err(database_error)?;
    conn.authorizer(Some(authorize)).map_err(database_error)?;
    conn.execute_batch(text.trim_start_matches('\u{feff}'))
        .map_err(database_error)?;
    if !conn.is_autocommit() {
        return Err(ApiError::invalid("SQL 导出事务未完成，请重新导出完整文件"));
    }
    conn.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .map_err(database_error)?;
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(database_error)?;
    if version != 20 && version != 0 {
        return Err(ApiError::invalid(format!(
            "暂不支持 CCS schema {version}，当前适配 schema 20"
        )));
    }
    columns(
        &conn,
        "providers",
        &[
            "id",
            "app_type",
            "name",
            "settings_config",
            "meta",
            "is_current",
            "in_failover_queue",
        ],
    )?;
    columns(
        &conn,
        "provider_endpoints",
        &["id", "provider_id", "app_type", "url"],
    )?;
    columns(
        &conn,
        "proxy_request_logs",
        &[
            "request_id",
            "provider_id",
            "app_type",
            "model",
            "request_model",
            "pricing_model",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "input_token_semantics",
            "data_source",
            "status_code",
            "created_at",
            "total_cost_usd",
        ],
    )?;
    columns(
        &conn,
        "usage_daily_rollups",
        &[
            "date",
            "app_type",
            "provider_id",
            "model",
            "request_model",
            "pricing_model",
            "request_count",
            "success_count",
            "input_tokens",
            "output_tokens",
            "cache_read_tokens",
            "cache_creation_tokens",
            "input_token_semantics",
            "total_cost_usd",
        ],
    )?;
    if version == 0 {
        columns(
            &conn,
            "session_usage_dedup",
            &["data_source", "request_id", "semantic_id", "has_entry_id"],
        )?;
    }
    let mut providers = conn
        .prepare("SELECT id,app_type,name,settings_config,meta FROM providers ORDER BY app_type,id")
        .map_err(database_error)?;
    let rows = providers
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(database_error)?;
    let mut candidates = Vec::new();
    for row in rows {
        let (provider, app, name, settings, meta) = row.map_err(database_error)?;
        if candidates.len() >= 1000 || provider.len() > 256 || app.len() > 64 || name.len() > 256 {
            return Err(ApiError::invalid("来源供应商数量或字段长度超过支持范围"));
        }
        let settings: Value = serde_json::from_str(&settings)
            .map_err(|_| ApiError::invalid("供应商配置 JSON 无效"))?;
        let meta: Value =
            serde_json::from_str(&meta).map_err(|_| ApiError::invalid("供应商元信息 JSON 无效"))?;
        let mut candidate = connection(provider, app, name, &settings, &meta);
        let mut endpoints = conn.prepare("SELECT url FROM provider_endpoints WHERE provider_id=?1 AND app_type=?2 ORDER BY id").map_err(database_error)?;
        candidate.endpoints = endpoints
            .query_map([&candidate.provider, &candidate.app], |r| {
                r.get::<_, String>(0)
            })
            .map_err(database_error)?
            .collect::<std::result::Result<_, _>>()
            .map_err(database_error)?;
        candidates.push(candidate);
    }
    let mut usage = usage(&conn)?;
    if version == 0 {
        usage
            .warnings
            .push("导出未声明 schema 版本，已核对 schema 20 兼容结构".into());
    }
    Ok(ParsedSource { candidates, usage })
}
pub fn normalized_url(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw).map_err(|_| ApiError::invalid("上游地址无效"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.len() > 2048
    {
        return Err(ApiError::invalid(
            "上游地址须为不含凭据、查询参数或片段的 HTTP 地址",
        ));
    }
    Ok(url.as_str().trim_end_matches('/').into())
}
pub fn display_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(url) => format!(
            "{}{}",
            url.origin().ascii_serialization(),
            if url.path() == "/" { "/" } else { "/…" }
        ),
        Err(_) => "地址格式无效".into(),
    }
}
fn connection(
    provider: String,
    app: String,
    name: String,
    settings: &Value,
    meta: &Value,
) -> Candidate {
    let mut item = Candidate {
        id: Uuid::new_v4().to_string(),
        provider,
        app,
        name,
        base_url: None,
        api_key: None,
        protocol: None,
        model: None,
        endpoints: vec![],
        reason: None,
    };
    let result = (|| -> Result<()> {
        item.protocol = meta
            .get("apiFormat")
            .or_else(|| meta.get("api_format"))
            .or_else(|| settings.get("api_format"))
            .or_else(|| settings.get("apiFormat"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (address, key) = match item.app.as_str() {
            "codex" => {
                let text = settings
                    .get("config")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ApiError::invalid("缺少 Codex config.toml 配置"))?;
                let document: toml::Value = toml::from_str(text).map_err(|_| {
                    ApiError::invalid("无法解析 Codex config.toml，请检查来源配置的 TOML 语法")
                })?;
                let config = document
                    .get("model_provider")
                    .and_then(toml::Value::as_str)
                    .and_then(|selected| document.get("model_providers")?.get(selected));
                item.protocol = config
                    .and_then(|v| v.get("wire_api"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .or(item.protocol.take());
                item.model = document
                    .get("model")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned);
                let address = config
                    .and_then(|v| v.get("base_url"))
                    .or_else(|| document.get("base_url"))
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned);
                let key = settings
                    .pointer("/auth/OPENAI_API_KEY")
                    .and_then(Value::as_str)
                    .filter(|key| !key.trim().is_empty())
                    .or_else(|| {
                        config
                            .and_then(|v| v.get("experimental_bearer_token"))
                            .or_else(|| document.get("experimental_bearer_token"))
                            .and_then(toml::Value::as_str)
                    })
                    .map(str::to_owned);
                (address, key)
            }
            "claude" => (
                settings
                    .pointer("/env/ANTHROPIC_BASE_URL")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                settings
                    .pointer("/env/ANTHROPIC_AUTH_TOKEN")
                    .or_else(|| settings.pointer("/env/ANTHROPIC_API_KEY"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ),
            "gemini" => (
                Some(
                    settings
                        .pointer("/env/GOOGLE_GEMINI_BASE_URL")
                        .and_then(Value::as_str)
                        .unwrap_or("https://generativelanguage.googleapis.com")
                        .to_owned(),
                ),
                settings
                    .pointer("/env/GEMINI_API_KEY")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            ),
            _ => return Err(ApiError::invalid("暂不能识别该应用的 API 地址与 Key 配置")),
        };
        let address_result = address
            .as_deref()
            .ok_or_else(|| ApiError::invalid("缺少上游 API 地址"))
            .and_then(|raw| normalized_url(raw.trim()));
        if let Ok(address) = &address_result {
            item.base_url = Some(address.clone());
        }
        let key =
            key.ok_or_else(|| ApiError::invalid("缺少 API Key，OAuth 配置不能作为 Key 导入"))?;
        let key = key.trim();
        if key.is_empty() || key.len() > 16384 || !key.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ApiError::invalid("API Key 格式无效"));
        }
        item.api_key = Some(key.into());
        address_result?;
        Ok(())
    })();
    if let Err(error) = result {
        item.reason = Some(error.message);
    }
    item
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageGroup {
    pub granularity: String,
    pub app_type: String,
    pub provider_id: String,
    pub model: String,
    pub request_model: Option<String>,
    pub pricing_model: Option<String>,
    pub data_sources: Vec<Option<String>>,
    pub date: Option<String>,
    pub first_time_seconds: Option<String>,
    pub last_time_seconds: Option<String>,
    pub requests: String,
    pub successes: String,
    pub input_tokens: Option<String>,
    pub raw_input_tokens: Option<String>,
    pub output_tokens: Option<String>,
    pub cache_read_tokens: Option<String>,
    pub cache_write_tokens: Option<String>,
    pub total_tokens: Option<String>,
    pub original_cost_usd: Option<String>,
    pub input_token_semantics: Vec<i64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsagePreview {
    pub detail_rows: String,
    pub excluded_duplicates: String,
    pub rollup_rows: String,
    pub groups: Vec<UsageGroup>,
    pub warnings: Vec<String>,
}
struct Aggregate {
    requests: u128,
    successes: u128,
    tokens: [Option<u128>; 4],
    raw_input: Option<u128>,
    cost: Option<Decimal>,
    first: Option<i64>,
    last: Option<i64>,
    semantics: HashSet<i64>,
    data_sources: HashSet<Option<String>>,
}
impl Default for Aggregate {
    fn default() -> Self {
        Self {
            requests: 0,
            successes: 0,
            tokens: [Some(0); 4],
            raw_input: Some(0),
            cost: Some(Decimal::ZERO),
            first: None,
            last: None,
            semantics: HashSet::new(),
            data_sources: HashSet::new(),
        }
    }
}
fn add_tokens(old: &mut Option<u128>, value: Option<i64>) {
    *old = old.and_then(|n| {
        value
            .filter(|v| *v >= 0)
            .and_then(|v| n.checked_add(v as u128))
    });
}
fn fresh(
    app: &str,
    input: Option<i64>,
    read: Option<i64>,
    write: Option<i64>,
    semantics: i64,
) -> Option<i64> {
    if !(0..=2).contains(&semantics) {
        return None;
    }
    let input = input.filter(|v| *v >= 0)?;
    if !matches!(app, "codex" | "gemini" | "grokbuild") || semantics == 2 {
        return Some(input);
    }
    let read = read.filter(|v| *v >= 0)?;
    let deduct = if semantics == 1 {
        read.checked_add(write.filter(|v| *v >= 0)?)?
    } else {
        read
    };
    Some(if input >= deduct {
        input - deduct
    } else {
        input
    })
}
const EFFECTIVE: &str = "NOT (COALESCE(l.data_source,'proxy') IN ('session_log','codex_session','gemini_session','opencode_session') AND EXISTS (SELECT 1 FROM proxy_request_logs p WHERE COALESCE(p.data_source,'proxy')='proxy' AND p.app_type IN (l.app_type,CASE WHEN l.app_type='claude' THEN 'claude-desktop' ELSE l.app_type END) AND p.status_code>=200 AND p.status_code<300 AND p.input_tokens=l.input_tokens AND p.output_tokens=l.output_tokens AND p.cache_read_tokens=l.cache_read_tokens AND (p.cache_creation_tokens=l.cache_creation_tokens OR (l.cache_creation_tokens=0 AND COALESCE(l.data_source,'proxy') IN ('codex_session','gemini_session','opencode_session')) OR (p.cache_creation_tokens=0 AND p.input_token_semantics=0 AND COALESCE(l.data_source,'proxy')='codex_session')) AND p.created_at BETWEEN l.created_at-600 AND l.created_at+600 AND (LOWER(p.model)=LOWER(l.model) OR LOWER(p.model)='unknown' OR LOWER(l.model)='unknown')))";
fn usage(conn: &Connection) -> Result<UsagePreview> {
    let original: i64 = conn
        .query_row("SELECT COUNT(*) FROM proxy_request_logs", [], |r| r.get(0))
        .map_err(database_error)?;
    let mut groups: BTreeMap<
        (
            String,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        Aggregate,
    > = BTreeMap::new();
    let mut detail_rows = 0u128;
    let mut rollup_rows = 0u128;
    for rollup in [false, true] {
        let sql = if rollup {
            "SELECT app_type,provider_id,model,date,NULL,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd,request_count,success_count,request_model,pricing_model,NULL FROM usage_daily_rollups".into()
        } else {
            format!(
                "SELECT app_type,provider_id,model,NULL,created_at,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,input_token_semantics,total_cost_usd,1,CASE WHEN status_code>=200 AND status_code<300 THEN 1 ELSE 0 END,request_model,pricing_model,data_source FROM proxy_request_logs l WHERE {EFFECTIVE}"
            )
        };
        let mut statement = conn.prepare(&sql).map_err(database_error)?;
        let mut rows = statement.query([]).map_err(database_error)?;
        while let Some(row) = rows.next().map_err(database_error)? {
            let app: String = row.get(0).map_err(database_error)?;
            let provider: String = row.get(1).map_err(database_error)?;
            let model: String = row.get(2).map_err(database_error)?;
            let date: Option<String> = row.get(3).map_err(database_error)?;
            let time: Option<i64> = row.get(4).map_err(database_error)?;
            let input: Option<i64> = row.get(5).map_err(database_error)?;
            let output: Option<i64> = row.get(6).map_err(database_error)?;
            let read: Option<i64> = row.get(7).map_err(database_error)?;
            let write: Option<i64> = row.get(8).map_err(database_error)?;
            let semantics: i64 = row.get(9).map_err(database_error)?;
            let cost: Option<String> = row.get(10).map_err(database_error)?;
            let requests: i64 = row.get(11).map_err(database_error)?;
            let successes: i64 = row.get(12).map_err(database_error)?;
            let request_model: Option<String> = row.get(13).map_err(database_error)?;
            let pricing_model: Option<String> = row.get(14).map_err(database_error)?;
            let data_source: Option<String> = row.get(15).map_err(database_error)?;
            if requests < 0 || successes < 0 || successes > requests {
                return Err(ApiError::invalid("来源请求计数无效"));
            }
            let normalized_input = fresh(&app, input, read, write, semantics);
            let group = groups
                .entry((
                    if rollup { "daily" } else { "detail" }.into(),
                    app,
                    provider,
                    model,
                    date,
                    request_model,
                    pricing_model,
                ))
                .or_default();
            group.requests += requests as u128;
            group.successes += successes as u128;
            add_tokens(&mut group.raw_input, input);
            for (old, value) in group
                .tokens
                .iter_mut()
                .zip([normalized_input, output, read, write])
            {
                add_tokens(old, value);
            }
            group.cost = match (group.cost, cost) {
                (Some(old), Some(value)) => old.checked_add(
                    Decimal::from_str(&value)
                        .map_err(|_| ApiError::invalid("来源金额不是有效十进制数字"))?,
                ),
                _ => None,
            };
            if let Some(time) = time {
                group.first = Some(group.first.map_or(time, |v| v.min(time)));
                group.last = Some(group.last.map_or(time, |v| v.max(time)));
            }
            group.semantics.insert(semantics);
            group.data_sources.insert(data_source);
            if rollup {
                rollup_rows += 1;
            } else {
                detail_rows += 1;
            }
            if groups.len() > 10000 {
                return Err(ApiError::invalid(
                    "来源用量分组超过 10000 项，请缩小导出范围",
                ));
            }
        }
    }
    let groups = groups
        .into_iter()
        .map(
            |(
                (granularity, app_type, provider_id, model, date, request_model, pricing_model),
                g,
            )| {
                let mut semantics: Vec<_> = g.semantics.into_iter().collect();
                semantics.sort();
                let mut data_sources: Vec<_> = g.data_sources.into_iter().collect();
                data_sources.sort();
                let total = g
                    .tokens
                    .into_iter()
                    .try_fold(0u128, |sum, n| sum.checked_add(n?))
                    .map(|n| n.to_string());
                UsageGroup {
                    granularity,
                    app_type,
                    provider_id,
                    model,
                    request_model,
                    pricing_model,
                    data_sources,
                    date,
                    first_time_seconds: g.first.map(|v| v.to_string()),
                    last_time_seconds: g.last.map(|v| v.to_string()),
                    requests: g.requests.to_string(),
                    successes: g.successes.to_string(),
                    input_tokens: g.tokens[0].map(|v| v.to_string()),
                    raw_input_tokens: g.raw_input.map(|v| v.to_string()),
                    output_tokens: g.tokens[1].map(|v| v.to_string()),
                    cache_read_tokens: g.tokens[2].map(|v| v.to_string()),
                    cache_write_tokens: g.tokens[3].map(|v| v.to_string()),
                    total_tokens: total,
                    original_cost_usd: g.cost.map(|v| v.normalize().to_string()),
                    input_token_semantics: semantics,
                }
            },
        )
        .collect();
    let mut warnings = vec!["费用为来源原值，币种按 CCS USD 字段保留，不代表供应商账单".into()];
    if rollup_rows > 0 {
        warnings.push(
            "日聚合未记录来源时区，无法确认与明细的日界覆盖，二者分开展示且不合并总量".into(),
        );
    }
    Ok(UsagePreview {
        detail_rows: detail_rows.to_string(),
        excluded_duplicates: (original as u128 - detail_rows).to_string(),
        rollup_rows: rollup_rows.to_string(),
        groups,
        warnings,
    })
}
