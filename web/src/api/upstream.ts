import { array, boolean, integer, nullable, object, request, text } from './index'

export type QueryType
  = | 'disabled'
    | 'cpr'
    | 'kimi'
    | 'kimi_global'
    | 'kimi_coding'
    | 'kimi_coding_global'
    | 'deepseek'
    | 'sub2api'

export const queryOptions: { label: string, value: QueryType }[] = [
  { label: '不查询', value: 'disabled' },
  { label: 'CPR 上游账号额度', value: 'cpr' },
  { label: 'Kimi 按量付费 · 国内', value: 'kimi' },
  { label: 'Kimi 按量付费 · 海外', value: 'kimi_global' },
  { label: 'Kimi 编程订阅 · 国内', value: 'kimi_coding' },
  { label: 'Kimi 编程订阅 · 海外', value: 'kimi_coding_global' },
  { label: 'DeepSeek 余额', value: 'deepseek' },
  { label: 'Sub2API 钱包与订阅', value: 'sub2api' },
]

export interface QuotaWindow {
  key: string
  label: string
  usedPercent: number | null
  resetAt: string | null
}

export interface QuotaAmount {
  label: string
  unit: string | null
  remaining: string | null
  used: string | null
  total: string | null
  unlimited: boolean
  resetAt: string | null
  isValid: boolean
  invalidMessage: string | null
  details: { label: string, amount: string | null }[]
}

export interface UpstreamAccount {
  id: string
  label: string
  models: string[] | null
  status: 'unknown' | 'observed' | 'stale'
  observedAt: string | null
  windows: QuotaWindow[]
  amounts: QuotaAmount[]
  message: string | null
}

export interface UpstreamState {
  version: number
  queryType: QueryType
  snapshot: { fetchedAt: string, accounts: UpstreamAccount[] } | null
  lastAttemptAt: string | null
  stale: boolean
  forwardingBlocked: boolean
  error: string | null
}

export interface QuotaSource {
  id: string
  name: string
  enabled: boolean
}

function invalid(): never {
  throw new Error('上游额度响应格式不正确，请检查插件版本')
}

function timestamp(value: unknown): string | null {
  const result = nullable(value)
  return result === null || Number.isFinite(Date.parse(result)) ? result : invalid()
}

function queryType(value: unknown): QueryType {
  return queryOptions.find(option => option.value === value)?.value ?? invalid()
}

function parseAccount(value: unknown): UpstreamAccount {
  const account = object(value)
  const status = text(account.status)
  if (status !== 'unknown' && status !== 'observed' && status !== 'stale')
    return invalid()
  return {
    id: text(account.id),
    label: text(account.label),
    status,
    observedAt: timestamp(account.observedAt),
    message: nullable(account.message),
    models: account.models === null ? null : array(account.models, text),
    windows: array(account.windows, (value) => {
      const window = object(value)
      const percent = window.usedPercent
      if (
        percent !== null
        && (typeof percent !== 'number' || !Number.isFinite(percent) || percent < 0 || percent > 100)
      ) {
        return invalid()
      }
      return {
        key: text(window.key),
        label: text(window.label),
        usedPercent: percent,
        resetAt: timestamp(window.resetAt),
      }
    }),
    amounts: array(account.amounts, (value) => {
      const entry = object(value)
      return {
        label: text(entry.label),
        unit: nullable(entry.unit),
        remaining: nullable(entry.remaining),
        used: nullable(entry.used),
        total: nullable(entry.total),
        unlimited: boolean(entry.unlimited),
        isValid: boolean(entry.isValid),
        invalidMessage: nullable(entry.invalidMessage),
        resetAt: timestamp(entry.resetAt),
        details: array(entry.details, (value) => {
          const detail = object(value)
          return { label: text(detail.label), amount: nullable(detail.amount) }
        }),
      }
    }),
  }
}

function parseState(value: unknown): UpstreamState {
  const state = object(value)
  const snapshot = state.snapshot === null ? null : object(state.snapshot)
  const fetchedAt = snapshot ? timestamp(snapshot.fetchedAt) : null
  if (snapshot && fetchedAt === null)
    return invalid()
  return {
    version: integer(state.version),
    queryType: queryType(state.queryType),
    snapshot: snapshot
      ? { fetchedAt: fetchedAt!, accounts: array(snapshot.accounts, parseAccount) }
      : null,
    lastAttemptAt: timestamp(state.lastAttemptAt),
    stale: boolean(state.stale),
    forwardingBlocked: boolean(state.forwardingBlocked),
    error: nullable(state.error),
  }
}

export function getQuotaSources() {
  return request('GET', 'api/upstream/accounts', null, value =>
    array(object(value).accounts, (value) => {
      const account = object(value)
      return { id: text(account.id), name: text(account.name), enabled: boolean(account.enabled) }
    }))
}

export function getUpstream(accountId: string) {
  return request('POST', 'api/upstream/get', { accountId }, parseState)
}
export function saveUpstream(accountId: string, expectedVersion: number, queryType: QueryType) {
  return request('POST', 'api/upstream/save', { accountId, expectedVersion, queryType }, parseState)
}
export function refreshUpstream(accountId: string) {
  return request('POST', 'api/upstream/refresh', { accountId }, parseState)
}

export interface QuotaSharing {
  configRevision: number
  enabled: boolean
  includeNames: boolean
  includeModels: boolean
}
function parseSharing(value: unknown): QuotaSharing {
  const data = object(value)
  return { configRevision: integer(data.configRevision), enabled: boolean(data.enabled), includeNames: boolean(data.includeNames), includeModels: boolean(data.includeModels) }
}
export function getQuotaSharing(clientKeyId: string) {
  return request('POST', 'api/upstream/sharing', { clientKeyId }, parseSharing)
}
export function saveQuotaSharing(clientKeyId: string, value: QuotaSharing) {
  return request('POST', 'api/upstream/sharing/save', {
    clientKeyId,
    expectedConfigRevision: value.configRevision,
    enabled: value.enabled,
    includeNames: value.includeNames,
    includeModels: value.includeModels,
  }, parseSharing)
}
