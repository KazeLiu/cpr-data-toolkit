//! 宿主聚合行的展示归并，不读取原始请求或重新计价
use super::source::{Group, Key};
use crate::api::{ApiError, Result};
use gateway_plugin_sdk::call::services::usage::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, str::FromStr};
pub(super) const MAX_ROWS: usize = 20000;
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    Model,
    Key,
    Group,
}
#[derive(Default, Clone)]
struct Metric {
    sum: i128,
    known: u64,
    missing: u64,
}
impl Metric {
    fn view(&self) -> Value {
        json!({"value":self.sum.to_string(),"known":self.known,"missing":self.missing})
    }
}
#[derive(Default, Clone)]
struct Amount {
    sum: Decimal,
    native: Option<Decimal>,
    contribution: Option<Decimal>,
}
fn money(value: &str) -> Result<Decimal> {
    Decimal::from_str(value).map_err(|_| ApiError::invalid("宿主费用无法按十进制读取"))
}
fn add(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_add(right)
        .ok_or_else(|| ApiError::invalid("汇总金额超出十进制表示范围"))
}
fn add_optional(left: Option<Decimal>, right: Option<Decimal>) -> Result<Option<Decimal>> {
    match (left, right) {
        (Some(a), Some(b)) => Ok(Some(add(a, b)?)),
        (a, b) => Ok(a.or(b)),
    }
}
#[derive(Default, Clone)]
pub(super) struct Totals {
    counts: [i128; 5],
    tokens: [Metric; 5],
    costs: BTreeMap<String, Amount>,
    unpriced: i128,
    contributions: u64,
    reported: u64,
    calculated: u64,
}
impl Totals {
    fn statistics(value: &UsageStatisticsMetrics) -> Result<Self> {
        let mut total = Self {
            unpriced: i128::from(value.cost_coverage.unavailable.get()),
            contributions: value.contribution_count.get(),
            reported: value.cost_coverage.provider_reported.get(),
            calculated: value.cost_coverage.calculated.get(),
            ..Self::default()
        };
        for (index, metric) in [
            UsageMetric::RequestCount,
            UsageMetric::SuccessCount,
            UsageMetric::FailureCount,
            UsageMetric::CancelledCount,
            UsageMetric::IncompleteCount,
        ]
        .iter()
        .enumerate()
        {
            total.counts[index] = value.values.get(metric).map_or(0, |v| i128::from(v.get()));
        }
        // 三种费用覆盖状态互斥，分母只含原生完整交付请求，不能使用所有请求数
        let eligible = total.reported + total.calculated + value.cost_coverage.unavailable.get();
        for (index, metric) in [
            UsageMetric::InputTokens,
            UsageMetric::OutputTokens,
            UsageMetric::CachedInputTokens,
            UsageMetric::CacheWriteInputTokens,
            UsageMetric::TotalTokens,
        ]
        .iter()
        .enumerate()
        {
            let native = value
                .native_token_samples
                .get(metric)
                .map_or(0, |v| v.get());
            let external = value
                .contribution_token_samples
                .get(metric)
                .map_or(0, |v| v.get());
            let missing = eligible
                .checked_sub(native)
                .and_then(|a| total.contributions.checked_sub(external).map(|b| a + b))
                .ok_or_else(|| {
                    ApiError::new(502, "statistics_coverage", "宿主返回的统计覆盖数量不一致")
                })?;
            total.tokens[index] = Metric {
                sum: value.values.get(metric).map_or(0, |v| i128::from(v.get())),
                known: native + external,
                missing,
            };
        }
        for cost in &value.costs {
            total.costs.insert(
                cost.currency.clone(),
                Amount {
                    sum: money(&cost.amount)?,
                    native: cost.native_amount.as_deref().map(money).transpose()?,
                    contribution: cost.contribution_amount.as_deref().map(money).transpose()?,
                },
            );
        }
        Ok(total)
    }
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (a, b) in self.counts.iter_mut().zip(other.counts) {
            *a += b;
        }
        for (a, b) in self.tokens.iter_mut().zip(&other.tokens) {
            a.sum += b.sum;
            a.known += b.known;
            a.missing += b.missing;
        }
        self.unpriced += other.unpriced;
        self.contributions += other.contributions;
        self.reported += other.reported;
        self.calculated += other.calculated;
        for (currency, value) in &other.costs {
            let amount = self.costs.entry(currency.clone()).or_default();
            amount.sum = add(amount.sum, value.sum)?;
            amount.native = add_optional(amount.native, value.native)?;
            amount.contribution = add_optional(amount.contribution, value.contribution)?;
        }
        Ok(())
    }
    pub fn view(&self) -> Result<Value> {
        let costs=self.costs.iter().map(|(currency,v)| json!({"currency":currency,"amount":v.sum.normalize().to_string(),"native":v.native.map(|a|a.normalize().to_string()),"contribution":v.contribution.map(|a|a.normalize().to_string())})).collect::<Vec<_>>();
        let input = &self.tokens[0];
        let read = &self.tokens[2];
        let ratio = (input.sum > 0
            && input.known > 0
            && read.known > 0
            && input.missing == 0
            && read.missing == 0)
            .then(|| read.sum as f64 / input.sum as f64);
        Ok(
            json!({"requests":self.counts[0].to_string(),"succeeded":self.counts[1].to_string(),"failed":self.counts[2].to_string(),"cancelled":self.counts[3].to_string(),"incomplete":self.counts[4].to_string(),"unknownOutcomes":(self.counts[0]-self.counts[1..].iter().sum::<i128>()).to_string(),"input":input.view(),"output":self.tokens[1].view(),"cacheRead":read.view(),"cacheWrite":self.tokens[3].view(),"tokens":self.tokens[4].view(),"cacheRatio":ratio,"costs":costs,"unpriced":self.unpriced.to_string(),"contributions":self.contributions,"reportedCount":self.reported,"calculatedCount":self.calculated}),
        )
    }
}
pub(super) struct Cell {
    group: UsageStatisticsGroup,
    totals: Totals,
}
#[derive(Default)]
pub(super) struct DataSet {
    pub summary: Option<Totals>,
    cells: Vec<Cell>,
    pub observation: Option<UsageReadObservation>,
}
impl DataSet {
    pub fn push(&mut self, row: UsageStatisticsRow) -> Result<()> {
        let totals = Totals::statistics(&row.metrics)?;
        if let Some(group) = row.group {
            if self.cells.len() >= MAX_ROWS {
                return Err(ApiError::invalid("聚合行超过 20000 项，请缩小时间范围"));
            }
            self.cells.push(Cell { group, totals });
        } else {
            self.summary = Some(totals);
        }
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.cells.len()
    }
    pub fn snapshot(&self) -> Value {
        json!(self.observation)
    }
}
#[derive(Default)]
pub(super) struct Report {
    pub keys: BTreeMap<String, Key>,
    pub groups: BTreeMap<String, Group>,
    pub models: DataSet,
    pub routing: DataSet,
    pub detail: Option<(String, DataSet)>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub task_id: String,
    pub dimension: Dimension,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub search: String,
    pub sort: Sort,
    #[serde(default)]
    pub ascending: bool,
    pub parent: Option<Parent>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parent {
    pub dimension: Dimension,
    pub id: String,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    Requests,
    Tokens,
    Cost,
}
pub(super) struct Row {
    pub id: String,
    pub name: String,
    pub totals: Totals,
    inspect: bool,
}
impl Row {
    pub fn view(&self) -> Result<Value> {
        Ok(
            json!({"id":self.id,"name":self.name,"totals":self.totals.view()?,"canInspect":self.inspect}),
        )
    }
}
fn identity(group: &UsageStatisticsGroup, dimension: Dimension) -> Result<String> {
    let value = match dimension {
        Dimension::Model => &group.model,
        Dimension::Key => &group.client_key_ref,
        Dimension::Group => &group.routing_group_ref,
    };
    // KeyModel 的外部未归属标记可能仅因另一维度缺失，按目标维度重新归类
    Ok(serde_json::to_string(&(
        value,
        group.unassigned_contribution && value.is_none(),
    ))?)
}
impl Report {
    pub fn group_filter(&self, query: &Query) -> Result<Option<String>> {
        let Some(parent) = query
            .parent
            .as_ref()
            .filter(|p| p.dimension == Dimension::Group)
        else {
            return Ok(None);
        };
        let cell = self
            .routing
            .cells
            .iter()
            .find(|c| identity(&c.group, Dimension::Group).is_ok_and(|id| id == parent.id))
            .ok_or_else(|| ApiError::invalid("路由分组不存在"))?;
        cell.group
            .routing_group_ref
            .clone()
            .map(Some)
            .ok_or_else(|| ApiError::invalid("未关联分组和外部未归属项不支持分组交叉筛选"))
    }
    fn data(&self, query: &Query) -> Result<&DataSet> {
        if let Some(parent) = query
            .parent
            .as_ref()
            .filter(|p| p.dimension == Dimension::Group)
        {
            return self
                .detail
                .as_ref()
                .filter(|(id, _)| id == &parent.id)
                .map(|(_, data)| data)
                .ok_or_else(|| ApiError::conflict("分组明细尚未读取"));
        }
        Ok(if query.dimension == Dimension::Group {
            &self.routing
        } else {
            &self.models
        })
    }
    pub fn snapshot(&self, query: &Query) -> Result<Value> {
        Ok(self.data(query)?.snapshot())
    }
    pub fn rows(&self, query: &Query) -> Result<(Totals, Vec<Row>)> {
        if query.search.len() > 1024
            || (query.dimension == Dimension::Group && query.parent.is_some())
        {
            return Err(ApiError::invalid("统计筛选无效"));
        }
        let data = self.data(query)?;
        let filtered = query
            .parent
            .as_ref()
            .filter(|p| p.dimension != Dimension::Group);
        let mut summary = if filtered.is_none() {
            data.summary
                .clone()
                .ok_or_else(|| ApiError::conflict("统计尚未完整结束"))?
        } else {
            Totals::default()
        };
        let mut rows = BTreeMap::<String, Row>::new();
        for cell in &data.cells {
            if let Some(parent) = filtered {
                if identity(&cell.group, parent.dimension)? != parent.id {
                    continue;
                }
                summary.merge(&cell.totals)?;
            }
            let id = identity(&cell.group, query.dimension)?;
            let value = match query.dimension {
                Dimension::Model => &cell.group.model,
                Dimension::Key => &cell.group.client_key_ref,
                Dimension::Group => &cell.group.routing_group_ref,
            };
            let external = cell.group.unassigned_contribution && value.is_none();
            let name = if external {
                "外部贡献（未归属）".into()
            } else {
                match query.dimension {
                    Dimension::Model => value.clone().unwrap_or_else(|| "未归属模型".into()),
                    Dimension::Key => value
                        .as_ref()
                        .map(|id| {
                            self.keys
                                .get(id)
                                .map(|k| k.name.clone())
                                .unwrap_or_else(|| "已删除 Key".into())
                        })
                        .unwrap_or_else(|| "未归属 Key".into()),
                    Dimension::Group => value
                        .as_ref()
                        .map(|id| {
                            self.groups
                                .get(id)
                                .map(|g| g.name.clone())
                                .unwrap_or_else(|| "已删除分组".into())
                        })
                        .unwrap_or_else(|| "未关联路由分组".into()),
                }
            };
            let row = rows.entry(id.clone()).or_insert_with(|| Row {
                id,
                name,
                totals: Totals::default(),
                inspect: query.dimension != Dimension::Group || value.is_some(),
            });
            row.totals.merge(&cell.totals)?;
        }
        let search = query.search.trim().to_lowercase();
        let mut rows = rows
            .into_values()
            .filter(|row| {
                search.is_empty()
                    || format!("{} {}", row.name, row.id)
                        .to_lowercase()
                        .contains(&search)
            })
            .collect::<Vec<_>>();
        rows.sort_by(|a, b| {
            let rank = match query.sort {
                Sort::Requests => a.totals.counts[0].cmp(&b.totals.counts[0]),
                Sort::Tokens => a.totals.tokens[4].sum.cmp(&b.totals.tokens[4].sum),
                Sort::Cost => a
                    .totals
                    .costs
                    .get("USD")
                    .map_or(Decimal::ZERO, |v| v.sum)
                    .cmp(&b.totals.costs.get("USD").map_or(Decimal::ZERO, |v| v.sum)),
            };
            (if query.ascending {
                rank
            } else {
                rank.reverse()
            })
            .then_with(|| a.id.cmp(&b.id))
        });
        Ok((summary, rows))
    }
}
