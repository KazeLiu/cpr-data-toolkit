export interface PluginHost {
  readonly version: 2
  readonly theme: 'light' | 'dark'
  request: (input: {
    method: 'GET' | 'POST'
    path: string
    contentType?: string
    body?: string
  }) => Promise<{ status: number; contentType: string; body: ArrayBuffer }>
}
declare global {
  interface Window {
    codexProxyPlugin?: PluginHost
  }
}

export function host(): PluginHost {
  const value = window.codexProxyPlugin
  if (!value || value.version !== 2) throw new Error('请从 CPR 插件管理页面打开')
  return value
}
export class ApiError extends Error {
  constructor(
    message: string,
    public readonly status: number,
  ) {
    super(message)
  }
}
function invalid(): never {
  throw new Error('插件响应格式不正确，请检查插件版本')
}
export function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return invalid()
  return value as Record<string, unknown>
}
export function text(value: unknown): string {
  return typeof value === 'string' ? value : invalid()
}
export function nullable(value: unknown): string | null {
  return value === null ? null : text(value)
}
export function integer(value: unknown): number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : invalid()
}
export function boolean(value: unknown): boolean {
  return typeof value === 'boolean' ? value : invalid()
}
export function array<T>(value: unknown, parse: (value: unknown) => T): T[] {
  return Array.isArray(value) ? value.map(parse) : invalid()
}
export function version(value: unknown): number | null {
  return value === null ? null : integer(value)
}
export async function request<T>(
  method: 'GET' | 'POST',
  path: string,
  body: unknown,
  parse: (value: unknown) => T,
): Promise<T> {
  const response = await host().request({
    method,
    path,
    ...(method === 'POST' ? { contentType: 'application/json', body: JSON.stringify(body) } : {}),
  })
  const value: unknown = JSON.parse(new TextDecoder().decode(response.body))
  if (response.status >= 400) {
    const error = object(object(value).error)
    throw new ApiError(text(error.message), response.status)
  }
  return parse(value)
}
export function json(method: 'GET' | 'POST', path: string, body?: unknown) {
  return request(method, path, body, object)
}
export function message(error: unknown): string {
  return error instanceof Error ? error.message : '操作未完成'
}

export interface Rule {
  id: string
  enabled: boolean
  keyword: string
  userAgent: string
  originator: string | null
  version: string | null
}
function rule(value: unknown): Rule {
  const r = object(value)
  return {
    id: text(r.id),
    enabled: boolean(r.enabled),
    keyword: text(r.keyword),
    userAgent: text(r.userAgent),
    originator: nullable(r.originator),
    version: nullable(r.version),
  }
}
export const getRules = () =>
  request('GET', 'api/ua', undefined, (value) => {
    const r = object(value)
    return {
      version: version(r.version),
      rules: array(object(r.value).rules, rule),
    }
  })
export interface Key {
  id: string
  name: string
  enabled: boolean
}
export const getKeys = () =>
  request('GET', 'api/keys', undefined, (value) =>
    array(object(value).keys, (value) => {
      const key = object(value)
      return {
        id: text(key.id),
        name: text(key.name),
        enabled: boolean(key.enabled),
      }
    }),
  )
export const getModels = (clientKeyId: string) =>
  request('POST', 'api/models', { clientKeyId }, (value) => array(object(value).models, text))

export interface Header {
  name: string
  value: string
}
export const previewRules = (model: string, rules: Rule[]) =>
  request('POST', 'api/ua/preview', { model, value: { rules } }, (value) => {
    const r = object(value)
    return {
      ruleId: nullable(r.ruleId),
      headers: array(r.headers, (value) => ({
        name: text(object(value).name),
        value: text(object(value).value),
      })),
    }
  })
export const saveRules = (expectedVersion: number | null, rules: Rule[]) =>
  request('POST', 'api/ua/save', { expectedVersion, value: { rules } }, (value) =>
    integer(object(value).version),
  )
export const diagnose = (clientKeyId: string, model: string, baselineUa: string | null) =>
  request('POST', 'api/ua/diagnose', { clientKeyId, model, baselineUa }, (value) => {
    const r = object(value)
    if (boolean(r.outboundSent)) throw new Error('安全验证返回了异常发送状态')
    return {
      ruleId: nullable(r.ruleId),
      baselineKnown: boolean(r.baselineKnown),
      headers: array(r.headers, (value) => ({
        name: text(object(value).name),
        value: text(object(value).value),
      })),
      stages: array(r.stages, text),
      notCovered: array(r.notCovered, text),
      message: text(r.message),
    }
  })

export interface IdentityHeaders {
  userAgent: string | null
  originator: string | null
  version: string | null
}
export type UaLogStatus = 'rewritten' | 'unmatched' | 'missing_model' | 'rules_error'
export interface UaRequestLog {
  id: string
  timeMs: number
  requestId: string
  model: string | null
  status: UaLogStatus
  rule: { id: string; keyword: string } | null
  before: IdentityHeaders
  after: IdentityHeaders
  nextFailed: boolean
}
function identityHeaders(value: unknown): IdentityHeaders {
  const headers = object(value)
  return {
    userAgent: nullable(headers.userAgent),
    originator: nullable(headers.originator),
    version: nullable(headers.version),
  }
}
export const getUaLogs = (offset: number) =>
  request('POST', 'api/ua/logs', { offset }, (value) => {
    const page = object(value)
    return {
      total: integer(page.total),
      capacity: integer(page.capacity),
      pageSize: integer(page.pageSize),
      entries: array(page.entries, (value): UaRequestLog => {
        const entry = object(value)
        const status = text(entry.status)
        if (
          status !== 'rewritten' &&
          status !== 'unmatched' &&
          status !== 'missing_model' &&
          status !== 'rules_error'
        )
          return invalid()
        const matched = entry.rule === null ? null : object(entry.rule)
        return {
          id: text(entry.id),
          timeMs: integer(entry.timeMs),
          requestId: text(entry.requestId),
          model: nullable(entry.model),
          status,
          rule: matched === null ? null : { id: text(matched.id), keyword: text(matched.keyword) },
          before: identityHeaders(entry.before),
          after: identityHeaders(entry.after),
          nextFailed: boolean(entry.nextFailed),
        }
      }),
    }
  })
export const clearUaLogs = () =>
  request('POST', 'api/ua/logs/clear', {}, (value) => boolean(object(value).cleared))

export interface Choice {
  id: string
  name: string
  kind: string
  accountId: string | null
  revision: number | null
}
export interface ImportGroup {
  id: string
  address: string
  status: string
  message: string
  choices: Choice[]
  targets: { accountId: string; revision: number }[]
  sources: {
    appType: string
    providerId: string
    protocol: string | null
    model: string | null
    endpoints: string[]
  }[]
}
export interface Unsupported {
  appType: string
  providerId: string
  name: string
  reason: string
}
export interface ImportSource {
  id: string
  groupId: string | null
  name: string
  appType: string
  address: string | null
  hasKey: boolean
  reason: string | null
}
export const previewImport = (sourceId: string) =>
  request('POST', 'api/import/preview', { sourceId }, (value) => {
    const r = object(value)
    return {
      note: text(r.note),
      rows: array(r.rows, (value): ImportSource => {
        const row = object(value)
        return {
          id: text(row.id),
          groupId: nullable(row.groupId),
          name: text(row.name),
          appType: text(row.appType),
          address: nullable(row.address),
          hasKey: boolean(row.hasKey),
          reason: nullable(row.reason),
        }
      }),
      groups: array(r.groups, (value): ImportGroup => {
        const g = object(value)
        return {
          id: text(g.id),
          address: text(g.address),
          status: text(g.status),
          message: text(g.message),
          choices: array(g.choices, (value) => {
            const c = object(value)
            return {
              id: text(c.id),
              name: text(c.name),
              kind: text(c.kind),
              accountId: nullable(c.accountId),
              revision: version(c.revision),
            }
          }),
          targets: array(g.targets, (value) => ({
            accountId: text(object(value).accountId),
            revision: integer(object(value).revision),
          })),
          sources: array(g.sources, (value) => {
            const s = object(value)
            return {
              appType: text(s.appType),
              providerId: text(s.providerId),
              protocol: nullable(s.protocol),
              model: nullable(s.model),
              endpoints: array(s.endpoints, text),
            }
          }),
        }
      }),
      unsupported: array(r.unsupported, (value): Unsupported => {
        const u = object(value)
        return {
          appType: text(u.appType),
          providerId: text(u.providerId),
          name: text(u.name),
          reason: text(u.reason),
        }
      }),
    }
  })
export const getImportKey = (sourceId: string, candidateId: string) =>
  request('POST', 'api/import/key', { sourceId, candidateId }, (value) => text(object(value).key))
export interface OperationResult {
  label: string
  status: string
  message: string
  accountId?: string
}
export function operationResults(value: unknown): OperationResult[] {
  return array(object(value).results, (value) => {
    const r = object(value)
    return {
      label: r.models ? array(r.models, text).join('、') : text(r.groupId),
      status: text(r.status),
      message: text(r.message),
      ...(typeof r.accountId === 'string' ? { accountId: r.accountId } : {}),
    }
  })
}
export interface UsageGroup {
  granularity: string
  appType: string
  providerId: string
  model: string
  requestModel: string | null
  pricingModel: string | null
  dataSources: (string | null)[]
  inputTokenSemantics: number[]
  date: string | null
  firstTimeSeconds: string | null
  lastTimeSeconds: string | null
  requests: string
  successes: string
  inputTokens: string | null
  rawInputTokens: string | null
  outputTokens: string | null
  cacheReadTokens: string | null
  cacheWriteTokens: string | null
  totalTokens: string | null
  originalCostUsd: string | null
}
export const getUsage = (sourceId: string, offset: number) =>
  request('POST', 'api/import/usage', { sourceId, offset }, (value) => {
    const r = object(value)
    return {
      total: integer(r.total),
      detailRows: text(r.detailRows),
      excludedDuplicates: text(r.excludedDuplicates),
      rollupRows: text(r.rollupRows),
      warnings: array(r.warnings, text),
      groups: array(r.groups, (value): UsageGroup => {
        const g = object(value)
        return {
          granularity: text(g.granularity),
          appType: text(g.appType),
          providerId: text(g.providerId),
          model: text(g.model),
          requestModel: nullable(g.requestModel),
          pricingModel: nullable(g.pricingModel),
          dataSources: array(g.dataSources, nullable),
          inputTokenSemantics: array(g.inputTokenSemantics, (value) =>
            typeof value === 'number' && Number.isSafeInteger(value) ? value : invalid(),
          ),
          date: nullable(g.date),
          firstTimeSeconds: nullable(g.firstTimeSeconds),
          lastTimeSeconds: nullable(g.lastTimeSeconds),
          requests: text(g.requests),
          successes: text(g.successes),
          inputTokens: nullable(g.inputTokens),
          rawInputTokens: nullable(g.rawInputTokens),
          outputTokens: nullable(g.outputTokens),
          cacheReadTokens: nullable(g.cacheReadTokens),
          cacheWriteTokens: nullable(g.cacheWriteTokens),
          totalTokens: nullable(g.totalTokens),
          originalCostUsd: nullable(g.originalCostUsd),
        }
      }),
    }
  })
export const getSummary = (period: string) =>
  request('POST', 'api/usage/summary', { period }, (value) => {
    const r = object(value)
    const tokens = text(r.tokens)
    const requests = text(r.requests)
    if (!/^\d+$/.test(tokens) || !/^\d+$/.test(requests)) return invalid()
    return { tokens, requests, period: text(r.period) }
  })

export interface TokenPrice {
  input: string
  output: string
  cacheRead: string
  cacheWrite: string
}
export interface Price {
  multiplierBps: number
  bands: Record<string, TokenPrice>
}
export interface Catalog {
  defaults: Record<string, Record<string, Price>>
  overrides: Record<string, Record<string, Price>>
  synced: Record<string, Record<string, Price>>
}
function price(value: unknown): Price {
  const r = object(value)
  return {
    multiplierBps: integer(r.multiplierBps),
    bands: Object.fromEntries(
      Object.entries(object(r.bands)).map(([key, value]) => {
        const p = object(value)
        return [
          key,
          {
            input: text(p.input),
            output: text(p.output),
            cacheRead: text(p.cacheRead),
            cacheWrite: text(p.cacheWrite),
          },
        ]
      }),
    ),
  }
}
export function prices(value: unknown) {
  return Object.fromEntries(
    Object.entries(object(value)).map(([provider, models]) => [
      provider,
      Object.fromEntries(
        Object.entries(object(models)).map(([model, value]) => [model, price(value)]),
      ),
    ]),
  )
}
export const getPricing = () =>
  request('GET', 'api/pricing', undefined, (value): Catalog => {
    const r = object(value)
    return {
      defaults: prices(r.defaults),
      synced: prices(r.synced),
      overrides: prices(r.overrides),
    }
  })
export interface CommunityPrice {
  sourceProvider: string
  model: string
  name: string
  input: string
  output: string
  cacheRead: string
  cacheWrite: string
}
export const getCommunity = (provider: string, offset: number, search = '', modelFilter = '') =>
  request('POST', 'api/pricing/community', { provider, offset, search, modelFilter }, (value) => {
    const r = object(value)
    return {
      providers: array(r.providers, (value) => ({
        id: text(object(value).id),
        name: text(object(value).name),
      })),
      total: integer(r.total),
      skipped: integer(r.skipped),
      rows: array(r.rows, (value): CommunityPrice => {
        const p = object(value)
        return {
          sourceProvider: text(p.sourceProvider),
          model: text(p.model),
          name: text(p.name),
          input: text(p.input),
          output: text(p.output),
          cacheRead: text(p.cacheRead),
          cacheWrite: text(p.cacheWrite),
        }
      }),
    }
  })
