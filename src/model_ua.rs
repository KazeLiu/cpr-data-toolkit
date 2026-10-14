use crate::{
    api::{ApiError, Result},
    state,
};
use gateway_plugin_sdk::{
    PluginFault,
    call::middleware::{MiddlewareHeader, MiddlewareHeaderMutation, MiddlewareRequestHead},
    client::{HostClient, MiddlewareResponse, RequestCall},
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
    after: IdentityHeaders,
    next_failed: bool,
}
impl RequestLog {
    fn new(head: &MiddlewareRequestHead) -> Self {
        let before = IdentityHeaders::capture(&head.headers);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must be after the Unix epoch")
                .as_millis() as u64,
            request_id: head.request_id.chars().take(128).collect(),
            model: head
                .model
                .as_ref()
                .map(|model| model.chars().take(256).collect()),
            status: LogStatus::MissingModel,
            rule: None,
            after: before.clone(),
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
    async fn mark_next_failed(&self, id: &str) {
        if let Some(entry) = self
            .entries
            .lock()
            .await
            .iter_mut()
            .find(|entry| entry.id == id)
        {
            entry.next_failed = true;
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
pub async fn middleware(
    logs: &RequestLogs,
    call: RequestCall,
) -> std::result::Result<MiddlewareResponse, PluginFault> {
    let mut request = call.request;
    let mut entry = RequestLog::new(&request.head);
    if request.head.model.is_some() {
        let config = match load(&call.host).await {
            Ok((_, config)) => config,
            Err(error) => {
                entry.status = LogStatus::RulesError;
                logs.record(entry).await;
                return Err(error);
            }
        };
        entry.status = LogStatus::Unmatched;
        if let Some((rule, mutations)) = config.mutations(request.head.model.as_deref()) {
            entry.rule = Some(MatchedRule {
                id: rule.id.clone(),
                keyword: rule.keyword.clone(),
            });
            entry.status = LogStatus::Rewritten;
            // 必须使用 SDK 增量接口，直接替换 head.headers 会连带重发未配置的入站身份头。
            for mutation in mutations {
                match mutation {
                    MiddlewareHeaderMutation::Remove { name } => request.remove_header(name),
                    MiddlewareHeaderMutation::Append { name, value } => {
                        request.append_header(name, value)
                    }
                }
            }
        }
    }
    entry.after = IdentityHeaders::capture(&request.head.headers);
    let id = entry.id.clone();
    logs.record(entry).await;
    let result = call.next.run(request).await;
    if result.is_err() {
        logs.mark_next_failed(&id).await;
    }
    result
}
