//! 模型、Key 和当前分组的精确汇总，保留缓存未知值与多币种费用

use super::source::Key;
use crate::api::{ApiError, Result};
use gateway_plugin_sdk::call::services::usage::*;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    str::FromStr,
};

const MAX_CELLS: usize = 20000;

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
    fn add(&mut self, value: Option<i128>) {
        if let Some(value) = value {
            self.sum += value;
            self.known += 1;
        } else {
            self.missing += 1;
        }
    }
    fn merge(&mut self, other: &Self) {
        self.sum += other.sum;
        self.known += other.known;
        self.missing += other.missing;
    }
    fn view(&self) -> Value {
        json!({"value":self.sum.to_string(),"known":self.known,"missing":self.missing})
    }
}

#[derive(Default, Clone)]
struct Amount {
    reported: Decimal,
    calculated: Decimal,
    contribution: Decimal,
}

fn add_money(left: Decimal, right: Decimal) -> Result<Decimal> {
    left.checked_add(right)
        .ok_or_else(|| ApiError::invalid("汇总金额超出十进制表示范围"))
}

#[derive(Default, Clone)]
pub(super) struct Totals {
    counts: [i128; 6],
    tokens: [Metric; 5],
    costs: BTreeMap<String, Amount>,
    unpriced: i128,
    contributions: u64,
    unknown_outcomes: i128,
}

impl Totals {
    pub(super) fn record(record: &UsageRecordData) -> Result<Self> {
        let mut total = Self::default();
        total.counts[0] = 1;
        total.counts[match record.outcome {
            UsageOutcome::Succeeded => 1,
            UsageOutcome::Failed => 2,
            UsageOutcome::Cancelled => 3,
            UsageOutcome::Incomplete => 4,
            UsageOutcome::Running => 5,
        }] = 1;
        if record.outcome != UsageOutcome::Succeeded {
            return Ok(total);
        }
        let tokens = &record.tokens;
        let input = tokens.input_tokens.map(|v| i128::from(v.get()));
        let output = tokens.output_tokens.map(|v| i128::from(v.get()));
        for (metric, value) in total.tokens.iter_mut().zip([
            input,
            output,
            tokens.cached_input_tokens.map(|v| i128::from(v.get())),
            tokens.cache_write_input_tokens.map(|v| i128::from(v.get())),
            tokens
                .total_tokens
                .map(|v| i128::from(v.get()))
                .or_else(|| input.zip(output).map(|(a, b)| a + b)),
        ]) {
            metric.add(value);
        }
        match &record.cost {
            UsageCost::Unavailable => total.unpriced = 1,
            UsageCost::Known {
                source,
                amount,
                currency,
            } => {
                let value = Decimal::from_str(amount)
                    .map_err(|_| ApiError::invalid("宿主费用无法按十进制读取"))?;
                let amount = total.costs.entry(currency.clone()).or_default();
                match source {
                    UsageCostSource::ProviderReported => amount.reported = value,
                    UsageCostSource::Calculated => amount.calculated = value,
                }
            }
        }
        Ok(total)
    }

    pub(super) fn aggregate(value: &UsageAggregate) -> Result<Self> {
        let mut total = Self {
            contributions: 1,
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
            total.counts[index] = value.counts.get(metric).map_or(0, |v| i128::from(v.get()));
        }
        total.unknown_outcomes = total.counts[0] - total.counts[1..5].iter().sum::<i128>();
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
            total.tokens[index].add(value.counts.get(metric).map(|v| i128::from(v.get())));
        }
        for (currency, value) in &value.costs {
            total
                .costs
                .entry(currency.clone())
                .or_default()
                .contribution = Decimal::from_str(value)
                .map_err(|_| ApiError::invalid("聚合贡献费用无法按十进制读取"))?;
        }
        if value.costs.is_empty() {
            total.unpriced = total.counts[0].max(0);
        }
        Ok(total)
    }

    fn merge(&mut self, other: &Self) -> Result<()> {
        for (left, right) in self.counts.iter_mut().zip(other.counts) {
            *left += right;
        }
        for (left, right) in self.tokens.iter_mut().zip(&other.tokens) {
            left.merge(right);
        }
        self.unpriced += other.unpriced;
        self.contributions += other.contributions;
        self.unknown_outcomes += other.unknown_outcomes;
        for (currency, value) in &other.costs {
            let amount = self.costs.entry(currency.clone()).or_default();
            amount.reported = add_money(amount.reported, value.reported)?;
            amount.calculated = add_money(amount.calculated, value.calculated)?;
            amount.contribution = add_money(amount.contribution, value.contribution)?;
        }
        Ok(())
    }

    pub(super) fn view(&self) -> Result<Value> {
        let costs = self.costs.iter().map(|(currency, value)| {
            Ok(json!({"currency":currency,"amount":add_money(add_money(value.reported,value.calculated)?,value.contribution)?.normalize().to_string(),
                "reported":value.reported.normalize().to_string(),"calculated":value.calculated.normalize().to_string(),"contribution":value.contribution.normalize().to_string()}))
        }).collect::<Result<Vec<_>>>()?;
        let input = &self.tokens[0];
        let read = &self.tokens[2];
        let ratio = if input.sum > 0 && input.missing == 0 && read.missing == 0 {
            Some(read.sum as f64 / input.sum as f64)
        } else {
            None
        };
        Ok(
            json!({"requests":self.counts[0].to_string(),"succeeded":self.counts[1].to_string(),"failed":self.counts[2].to_string(),
            "cancelled":self.counts[3].to_string(),"incomplete":self.counts[4].to_string(),"running":self.counts[5].to_string(),
            "input":input.view(),"output":self.tokens[1].view(),"cacheRead":read.view(),"cacheWrite":self.tokens[3].view(),"tokens":self.tokens[4].view(),
            "cacheRatio":ratio,"costs":costs,"unpriced":self.unpriced.to_string(),"contributions":self.contributions,"unknownOutcomes":self.unknown_outcomes.to_string()}),
        )
    }
}

#[derive(Default)]
struct Cell {
    totals: Totals,
    aliases: BTreeSet<String>,
}

#[derive(Default)]
pub(super) struct Report {
    cells: BTreeMap<(String, String, String), Cell>,
    pub keys: BTreeMap<String, Key>,
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
    pub provider: String,
    pub aliases: BTreeSet<String>,
    pub totals: Totals,
}

impl Row {
    pub fn view(&self) -> Result<Value> {
        Ok(
            json!({"id":self.id,"name":self.name,"provider":self.provider,"aliases":self.aliases,"totals":self.totals.view()?}),
        )
    }
}

impl Report {
    pub fn add(
        &mut self,
        provider: String,
        model: String,
        key: String,
        alias: Option<String>,
        totals: Totals,
    ) -> Result<()> {
        let identity = (provider, model, key);
        if !self.cells.contains_key(&identity) && self.cells.len() >= MAX_CELLS {
            return Err(ApiError::invalid(
                "模型与 Key 组合超过 20000 项，请缩小时间范围",
            ));
        }
        let cell = self.cells.entry(identity).or_default();
        cell.totals.merge(&totals)?;
        if let Some(alias) = alias {
            if !cell.aliases.contains(&alias) && cell.aliases.len() >= MAX_CELLS {
                return Err(ApiError::invalid("请求模型别名超过容量，请缩小时间范围"));
            }
            cell.aliases.insert(alias);
        }
        Ok(())
    }

    fn groups(&self, key: &str) -> Vec<(String, String)> {
        match self.keys.get(key) {
            None => vec![("unknown".into(), "未归属／已删除 Key".into())],
            Some(key) if key.groups.is_empty() => {
                vec![("all".into(), "未绑定分组（全部账号）".into())]
            }
            Some(key) => key
                .groups
                .iter()
                .map(|group| (group.id.clone(), group.name.clone()))
                .collect(),
        }
    }

    pub fn rows(&self, query: &Query) -> Result<(Totals, Vec<Row>)> {
        if query.search.len() > 1024 {
            return Err(ApiError::invalid("统计搜索内容过长"));
        }
        let mut summary = Totals::default();
        let mut rows = BTreeMap::<String, Row>::new();
        let search = query.search.trim().to_lowercase();
        for ((provider, model, key), cell) in &self.cells {
            let model_id = serde_json::to_string(&(provider, model))?;
            let groups = self.groups(key);
            if query
                .parent
                .as_ref()
                .is_some_and(|parent| match parent.dimension {
                    Dimension::Model => parent.id != model_id,
                    Dimension::Key => parent.id != *key,
                    Dimension::Group => !groups.iter().any(|(id, _)| *id == parent.id),
                })
            {
                continue;
            }
            summary.merge(&cell.totals)?;
            let identities = match query.dimension {
                Dimension::Model => vec![(
                    model_id,
                    if model.is_empty() {
                        "未归属模型".into()
                    } else {
                        model.clone()
                    },
                )],
                Dimension::Key => vec![(
                    key.clone(),
                    self.keys
                        .get(key)
                        .map(|k| k.name.clone())
                        .unwrap_or_else(|| {
                            if key.is_empty() {
                                "未归属 Key".into()
                            } else {
                                format!("已删除 Key · {key}")
                            }
                        }),
                )],
                Dimension::Group => groups,
            };
            for (id, name) in identities {
                if !rows.contains_key(&id) && rows.len() >= MAX_CELLS {
                    return Err(ApiError::invalid("分组结果超过 20000 项，请缩小时间范围"));
                }
                let row = rows.entry(id.clone()).or_insert_with(|| Row {
                    id,
                    name,
                    provider: if query.dimension == Dimension::Model {
                        provider.clone()
                    } else {
                        String::new()
                    },
                    aliases: BTreeSet::new(),
                    totals: Totals::default(),
                });
                row.totals.merge(&cell.totals)?;
                if query.dimension == Dimension::Model {
                    row.aliases.extend(cell.aliases.iter().cloned());
                }
            }
        }
        let mut rows = rows
            .into_values()
            .filter(|row| {
                search.is_empty()
                    || format!("{} {} {}", row.name, row.id, row.provider)
                        .to_lowercase()
                        .contains(&search)
            })
            .collect::<Vec<_>>();
        let mut ranked = rows
            .drain(..)
            .map(|row| {
                let amount = row
                    .totals
                    .costs
                    .get("USD")
                    .map(|v| add_money(add_money(v.reported, v.calculated)?, v.contribution))
                    .transpose()?
                    .unwrap_or_default();
                Ok((row, amount))
            })
            .collect::<Result<Vec<_>>>()?;
        ranked.sort_by(|(a, a_cost), (b, b_cost)| {
            let rank = match query.sort {
                Sort::Requests => a.totals.counts[0].cmp(&b.totals.counts[0]),
                Sort::Tokens => a.totals.tokens[4].sum.cmp(&b.totals.tokens[4].sum),
                // 多币种不换汇，金额排序只比较 USD；其他币种仍完整展示和导出
                Sort::Cost => a_cost.cmp(b_cost),
            };
            (if query.ascending {
                rank
            } else {
                rank.reverse()
            })
            .then_with(|| a.id.cmp(&b.id))
        });
        Ok((summary, ranked.into_iter().map(|(row, _)| row).collect()))
    }
}
