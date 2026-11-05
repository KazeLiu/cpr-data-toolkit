use crate::{
    api::{ApiError, Result},
    state,
};
use gateway_plugin_sdk::{
    PluginFault,
    call::middleware::{MiddlewareHeader, MiddlewareHeaderMutation},
    client::{HostClient, OutboundCall, OutboundRequest},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const LOG_CAPACITY: usize = 200;
const LOG_PAGE_SIZE: usize = 25;
const LOG_HEADER_BYTES: usize = 4096;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityHeaders {
    user_agent: Option<String>,
    originator: Option<String>,
    version: Option<String>,
}
impl IdentityHeaders {
    fn capture(headers: &[MiddlewareHeader]) -> Self {
        Self {
            user_agent: header_text(headers, "user-agent"),
            originator: header_text(headers, "originator"),
            version: header_text(headers, "version"),
        }
    }
}
fn header_text(headers: &[MiddlewareHeader], name: &str) -> Option<String> {
    let mut matching = headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name))
        .peekable();
    matching.peek()?;
    let mut bytes: Vec<_> = matching
        .enumerate()
        .flat_map(|(index, header)| {
            std::iter::once(b'\n')
                .take(usize::from(index != 0))
                .chain(header.value.iter().copied())
        })
        .take(LOG_HEADER_BYTES + 1)
        .collect();
    let truncated = bytes.len() > LOG_HEADER_BYTES;
    if truncated {
        bytes.truncate(LOG_HEADER_BYTES);
    }
    let mut value = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        value.push_str("\n[日志截断]");
    }
    Some(value)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStatus {
    Rewritten,
    Unmatched,
    MissingModel,
    RulesError,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MatchedRule {
    id: String,
    keyword: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLog {
    id: String,
    time_ms: u64,
    request_id: String,
    model: Option<String>,
    status: LogStatus,
    rule: Option<MatchedRule>,
    before: IdentityHeaders,
    after: Option<IdentityHeaders>,
    next_failed: bool,
}
impl RequestLog {
    fn new(request_id: &str, model: Option<&str>, headers: &[MiddlewareHeader]) -> Self {
        let before = IdentityHeaders::capture(headers);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_millis() as u64,
            request_id: request_id.chars().take(128).collect(),
            model: model.map(|model| model.chars().take(256).collect()),
            status: LogStatus::MissingModel,
            rule: None,
            after: None,
            before,
            next_failed: false,
        }
    }
}
#[derive(Default)]
pub struct RequestLogs {
    entries: Mutex<VecDeque<RequestLog>>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    entries: Vec<RequestLog>,
    total: usize,
    capacity: usize,
    page_size: usize,
}
impl RequestLogs {
    async fn record(&self, entry: RequestLog) {
        let mut entries = self.entries.lock().await;
        let index = entries
            .iter()
            .position(|previous| previous.time_ms <= entry.time_ms)
            .unwrap_or(entries.len());
        entries.insert(index, entry);
        if entries.len() > LOG_CAPACITY {
            entries.pop_back();
        }
    }
    pub async fn page(&self, offset: usize) -> Result<LogPage> {
        if offset >= LOG_CAPACITY || !offset.is_multiple_of(LOG_PAGE_SIZE) {
            return Err(ApiError::invalid("日志分页位置无效"));
        }
        let entries = self.entries.lock().await;
        Ok(LogPage {
            entries: entries
                .iter()
                .skip(offset)
                .take(LOG_PAGE_SIZE)
                .cloned()
                .collect(),
            total: entries.len(),
            capacity: LOG_CAPACITY,
            page_size: LOG_PAGE_SIZE,
        })
    }
    pub async fn clear(&self) {
        self.entries.lock().await.clear();
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub enabled: bool,
    pub keyword: String,
    pub user_agent: String,
    pub originator: Option<String>,
    pub version: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub rules: Vec<Rule>,
}
impl Configuration {
    pub fn validate(&self) -> Result<()> {
        if self.rules.len() > 100 {
            return Err(ApiError::invalid("最多保存 100 条规则"));
        }
        let mut ids = HashSet::new();
        for rule in &self.rules {
            if rule.id.is_empty()
                || rule.id.len() > 128
                || !ids.insert(&rule.id)
                || rule.keyword.trim().is_empty()
                || rule.keyword.len() > 256
            {
                return Err(ApiError::invalid("规则标识重复或模型关键词无效"));
            }
            for value in [
                Some(&rule.user_agent),
                rule.originator.as_ref(),
                rule.version.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                if value.is_empty()
                    || value.len() > 4096
                    || value.trim() != value
                    || !value.bytes().all(|b| (32..=126).contains(&b))
                    || http::HeaderValue::from_str(value).is_err()
                {
                    return Err(ApiError::invalid(
                        "身份头须为非空单行可见 ASCII 文本，最多 4096 字节",
                    ));
                }
            }
        }
        if serde_json::to_vec(self)?.len() > 128 * 1024 {
            return Err(ApiError::invalid("规则内容超过容量限制"));
        }
        Ok(())
    }
    fn mutations(&self, model: Option<&str>) -> Option<(&Rule, Vec<MiddlewareHeaderMutation>)> {
        let model = model?.to_lowercase();
        let rule = self
            .rules
            .iter()
            .find(|r| r.enabled && model.contains(&r.keyword.to_lowercase()))?;
        let mut mutations = Vec::new();
        for (name, value) in [
            ("user-agent", Some(&rule.user_agent)),
            ("originator", rule.originator.as_ref()),
            ("version", rule.version.as_ref()),
        ] {
            if let Some(value) = value {
                mutations.push(MiddlewareHeaderMutation::Remove { name: name.into() });
                mutations.push(MiddlewareHeaderMutation::Append {
                    name: name.into(),
                    value: value.as_bytes().to_vec(),
                });
            }
        }
        Some((rule, mutations))
    }
    pub fn rewrite<'a>(
        &'a self,
        model: Option<&str>,
        headers: &mut Vec<MiddlewareHeader>,
    ) -> Option<&'a Rule> {
        let (rule, mutations) = self.mutations(model)?;
        for mutation in mutations {
            match mutation {
                MiddlewareHeaderMutation::Remove { name } => {
                    headers.retain(|header| !header.name.eq_ignore_ascii_case(&name))
                }
                MiddlewareHeaderMutation::Append { name, value } => {
                    headers.push(MiddlewareHeader { name, value })
                }
            }
        }
        Some(rule)
    }
}
pub async fn load(
    host: &HostClient,
) -> std::result::Result<(Option<u64>, Configuration), PluginFault> {
    let (version, value) = state::get(host, "model_ua", "rules").await?;
    Ok((version, value.unwrap_or_default()))
}
pub async fn outbound(
    logs: &RequestLogs,
    pins: &crate::upstream_pin::Runtime,
    call: OutboundCall,
) -> std::result::Result<OutboundRequest, PluginFault> {
    // outbound 已完成宿主账号画像和默认头生成，不能用 request 阶段的入站头冒充
    let mut entry = RequestLog::new(
        &call.metadata.request_id,
        call.metadata.model.as_deref(),
        &call.request.head.headers,
    );
    let config = if entry.model.is_some() {
        match load(&call.host).await {
            Ok((_, config)) => config,
            Err(error) => {
                entry.status = LogStatus::RulesError;
                logs.record(entry).await;
                return Err(error);
            }
        }
    } else {
        Configuration::default()
    };
    if entry.model.is_some() {
        entry.status = LogStatus::Unmatched;
    }
    // 先完成后续出站准备（包括本插件的 DeepSeek 钉选），再应用 UA，返回实际准备好的头
    let mut request = match crate::upstream_pin::outbound(pins, call).await {
        Ok(request) => request,
        Err(error) => {
            entry.next_failed = true;
            logs.record(entry).await;
            return Err(error);
        }
    };
    if entry.model.is_some() {
        if let Some(rule) = config.rewrite(entry.model.as_deref(), &mut request.head.headers) {
            entry.rule = Some(MatchedRule {
                id: rule.id.clone(),
                keyword: rule.keyword.clone(),
            });
            entry.status = LogStatus::Rewritten;
        }
    }
    entry.after = Some(IdentityHeaders::capture(&request.head.headers));
    logs.record(entry).await;
    Ok(request)
}

// 自有 Cline 转换不经过原生 Provider 出站链，保留该路径的 UA 改写，但不伪造宿主画像日志
pub async fn adapter_headers(
    host: &HostClient,
    model: &str,
    headers: &mut Vec<(String, Vec<u8>)>,
) -> std::result::Result<(), PluginFault> {
    let (_, config) = load(host).await?;
    let mut values = headers
        .drain(..)
        .map(|(name, value)| MiddlewareHeader { name, value })
        .collect();
    config.rewrite(Some(model), &mut values);
    *headers = values
        .into_iter()
        .map(|header| (header.name, header.value))
        .collect();
    Ok(())
}
