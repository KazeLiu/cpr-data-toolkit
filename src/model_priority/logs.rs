//! 最近账号调度决定与宿主完成事实的有界关联，不保存凭据或请求正文

use crate::api::{ApiError, Result};
use gateway_plugin_sdk::{
    SendState,
    call::{
        observation::{RequestCompleted, RequestOutcome},
        policy::AccountScheduleRequest,
    },
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const CAPACITY: usize = 200;
const PAGE_SIZE: usize = 25;

#[derive(Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    Primary,
    Backup,
    NativeFallback,
    Unmatched,
    RulesError,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub attempt_index: u32,
    pub kind: DecisionKind,
    pub primary_account_id: Option<String>,
    pub selected_account_id: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    request_id: String,
    time_ms: u64,
    provider: String,
    model: Option<String>,
    decisions: Vec<Decision>,
    decisions_truncated: bool,
    completed: bool,
    actual_account_id: Option<String>,
    outcome: Option<RequestOutcome>,
    send_state: Option<SendState>,
    attempt_count: Option<u32>,
}

#[derive(Default)]
pub struct Logs {
    entries: Mutex<VecDeque<Entry>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    entries: Vec<Entry>,
    total: usize,
    capacity: usize,
    page_size: usize,
}

impl Logs {
    pub async fn record(&self, request: &AccountScheduleRequest, decision: Decision) {
        let mut entries = self.entries.lock().await;
        if !entries
            .iter()
            .any(|entry| entry.request_id == request.request_id)
        {
            entries.push_front(Entry {
                request_id: request.request_id.clone(),
                time_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock must be after Unix epoch")
                    .as_millis() as u64,
                provider: request.provider.clone(),
                model: request.model.clone(),
                decisions: vec![],
                decisions_truncated: false,
                completed: false,
                actual_account_id: None,
                outcome: None,
                send_state: None,
                attempt_count: None,
            });
            entries.truncate(CAPACITY);
        }
        let entry = entries
            .iter_mut()
            .find(|entry| entry.request_id == request.request_id)
            .expect("just inserted or found");
        // 同一 attempt 的容量重选只保留最新决定，不把等待轮询计作重试
        if let Some(previous) = entry
            .decisions
            .iter_mut()
            .find(|previous| previous.attempt_index == decision.attempt_index)
        {
            *previous = decision;
        } else {
            if entry.decisions.len() == 32 {
                entry.decisions.remove(0);
                entry.decisions_truncated = true;
            }
            entry.decisions.push(decision);
        }
    }

    pub async fn completed(&self, event: &RequestCompleted) {
        let mut entries = self.entries.lock().await;
        if !entries
            .iter()
            .any(|entry| entry.request_id == event.request_id)
        {
            entries.push_front(Entry {
                request_id: event.request_id.clone(),
                time_ms: event.completed_at_ms,
                provider: event.provider.clone().unwrap_or_default(),
                model: event
                    .upstream_model
                    .clone()
                    .or_else(|| event.requested_model.clone()),
                decisions: vec![],
                decisions_truncated: false,
                completed: false,
                actual_account_id: None,
                outcome: None,
                send_state: None,
                attempt_count: None,
            });
            entries.truncate(CAPACITY);
        }
        let entry = entries
            .iter_mut()
            .find(|entry| entry.request_id == event.request_id)
            .expect("just inserted or found");
        entry.completed = true;
        entry.actual_account_id = event.account_id.clone();
        entry.outcome = Some(event.terminal.outcome);
        entry.send_state = Some(event.terminal.send_state);
        entry.attempt_count = Some(event.terminal.attempt_count);
    }

    pub async fn page(&self, offset: usize) -> Result<Page> {
        if offset >= CAPACITY || !offset.is_multiple_of(PAGE_SIZE) {
            return Err(ApiError::invalid("日志分页位置无效"));
        }
        let entries = self.entries.lock().await;
        Ok(Page {
            entries: entries
                .iter()
                .skip(offset)
                .take(PAGE_SIZE)
                .cloned()
                .collect(),
            total: entries.len(),
            capacity: CAPACITY,
            page_size: PAGE_SIZE,
        })
    }

    pub async fn clear(&self) {
        self.entries.lock().await.clear();
    }
}
