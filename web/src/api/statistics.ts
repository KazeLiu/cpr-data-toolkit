import { array, boolean, integer, object, request, text } from './index'

export type Dimension = 'model' | 'key' | 'group'
export type StatSort = 'requests' | 'tokens' | 'cost'
export interface Metric { value: string, known: number, missing: number }
export interface Cost { currency: string, amount: string, reported: string, calculated: string, contribution: string }
export interface Totals {
  requests: string
  succeeded: string
  failed: string
  cancelled: string
  incomplete: string
  running: string
  unknownOutcomes: string
  input: Metric
  output: Metric
  cacheRead: Metric
  cacheWrite: Metric
  tokens: Metric
  cacheRatio: number | null
  costs: Cost[]
  unpriced: string
  contributions: number
}
export interface StatRow { id: string, name: string, provider: string, aliases: string[], totals: Totals }
export interface StatPage { summary: Totals, items: StatRow[], total: number, offset: number, pageSize: number }
export interface Parent { dimension: Dimension, id: string }
export interface StatQuery {
  taskId: string
  dimension: Dimension
  offset: number
  search: string
  sort: StatSort
  ascending: boolean
  parent: Parent | null
}
export interface StatTask {
  taskId: string
  step: number
  phase: 'keys' | 'records' | 'aggregates' | 'complete'
  scanned: number
  contributions: number
  keys: number
  timezone: string
  startMs: number
  endMs: number
  startDisplay: string
  endDisplay: string
  blocked: boolean
}
export interface Preset { id: string, name: string, range: string[] }

function count(value: unknown) {
  const result = text(value)
  if (!/^-?\d+$/.test(result)) throw new Error('统计计数格式无效')
  return result
}
function timestamp(value: unknown) {
  if (typeof value !== 'number' || !Number.isSafeInteger(value)) throw new Error('统计时间格式无效')
  return value
}
function metric(value: unknown): Metric {
  const data = object(value)
  return { value: count(data.value), known: integer(data.known), missing: integer(data.missing) }
}
function totals(value: unknown): Totals {
  const data = object(value)
  const ratio = data.cacheRatio
  if (ratio !== null && (typeof ratio !== 'number' || !Number.isFinite(ratio)))
    throw new Error('统计比例格式无效')
  return {
    requests: count(data.requests), succeeded: count(data.succeeded), failed: count(data.failed),
    cancelled: count(data.cancelled), incomplete: count(data.incomplete), running: count(data.running), unknownOutcomes: count(data.unknownOutcomes),
    input: metric(data.input), output: metric(data.output), cacheRead: metric(data.cacheRead),
    cacheWrite: metric(data.cacheWrite), tokens: metric(data.tokens), cacheRatio: ratio,
    unpriced: count(data.unpriced), contributions: integer(data.contributions),
    costs: array(data.costs, (value) => {
      const cost = object(value)
      return { currency: text(cost.currency), amount: text(cost.amount), reported: text(cost.reported),
        calculated: text(cost.calculated), contribution: text(cost.contribution) }
    }),
  }
}
function task(value: unknown): StatTask {
  const data = object(value)
  const phase = text(data.phase)
  if (phase !== 'keys' && phase !== 'records' && phase !== 'aggregates' && phase !== 'complete')
    throw new Error('统计进度格式无效')
  return { taskId: text(data.taskId), step: integer(data.step), phase, scanned: integer(data.scanned),
    contributions: integer(data.contributions), keys: integer(data.keys), timezone: text(data.timezone),
    startMs: timestamp(data.startMs), endMs: timestamp(data.endMs), startDisplay: text(data.startDisplay), endDisplay: text(data.endDisplay), blocked: boolean(data.blocked) }
}

export function getStatisticsMetadata(timezone: string) {
  return request('POST', 'api/statistics/metadata', { timezone }, (value) => {
    const data = object(value)
    return { presets: array(data.presets, (value): Preset => {
      const preset = object(value)
      return { id: text(preset.id), name: text(preset.name), range: array(preset.range, text) }
    }), timezones: array(data.timezones, text) }
  })
}
export const startStatistics = (timezone: string, startTime: string, endTime: string) =>
  request('POST', 'api/statistics/start', { timezone, startTime, endTime }, task)
export const stepStatistics = (taskId: string, step: number) =>
  request('POST', 'api/statistics/step', { taskId, step }, task)
export const cancelStatistics = (taskId: string) =>
  request('POST', 'api/statistics/cancel', { taskId }, value => boolean(object(value).cancelled))
export function queryStatistics(query: StatQuery) {
  return request('POST', 'api/statistics/query', query, (value): StatPage => {
    const data = object(value)
    return { summary: totals(data.summary), total: integer(data.total), offset: integer(data.offset), pageSize: integer(data.pageSize),
      items: array(data.items, (value) => {
        const row = object(value)
        return { id: text(row.id), name: text(row.name), provider: text(row.provider),
          aliases: array(row.aliases, text), totals: totals(row.totals) }
      }) }
  })
}
export const exportStatistics = (query: StatQuery) =>
  request('POST', 'api/statistics/export', query, (value) => {
    const data = object(value)
    return { filename: text(data.filename), csv: text(data.csv) }
  })

export function formatCount(value: string) {
  return BigInt(value).toLocaleString('zh-CN')
}
export function formatMetric(value: Metric) {
  if (value.missing > 0 && value.known === 0) return '未知'
  return `${formatCount(value.value)}${value.missing > 0 ? '（部分）' : ''}`
}
export function formatCosts(value: Cost[]) {
  return value.length ? value.map(cost => `${cost.currency} ${cost.amount}`).join(' / ') : '无已知费用'
}
export function costDetails(value: Totals) {
  return value.costs.map(cost => `${cost.currency}：上游回报 ${cost.reported}，宿主估算 ${cost.calculated}，聚合贡献 ${cost.contribution}`).join('\n')
}
export function quality(value: Totals) {
  const missing = [value.input, value.output, value.cacheRead, value.cacheWrite, value.tokens].reduce((sum, metric) => sum + metric.missing, 0)
  return [value.unpriced !== '0' ? `未计价请求／贡献 ${formatCount(value.unpriced)}` : '', missing ? `缺失指标项 ${missing}` : '', value.contributions ? `聚合贡献 ${value.contributions}` : ''].filter(Boolean).join(' · ')
}
