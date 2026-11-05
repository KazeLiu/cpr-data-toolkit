import { array, boolean, integer, nullable, object, request, text, version } from './index'

export interface PinRule {
  accountId: string
  model: string
  enabled: boolean
  providers: string[]
  has: string[]
}
function rule(value: unknown): PinRule {
  const row = object(value)
  return {
    accountId: text(row.accountId),
    model: text(row.model),
    enabled: boolean(row.enabled),
    providers: array(row.providers, text),
    has: array(row.has, text),
  }
}
function configuration(value: unknown) {
  const data = object(value)
  return { version: version(data.version), rules: array(data.rules, rule) }
}
export const getPins = () => request('GET', 'api/upstream-pin', null, configuration)
export function savePin(expectedVersion: number | null, rule: PinRule, originalModel?: string) {
  return request('POST', 'api/upstream-pin/save', { expectedVersion, rule, originalModel }, configuration)
}
export function deletePin(expectedVersion: number | null, rule: PinRule) {
  return request('POST', 'api/upstream-pin/delete', { expectedVersion, accountId: rule.accountId, model: rule.model }, configuration)
}

export interface PinLog {
  requestId: string
  attemptIndex: number
  timeMs: number
  accountId: string
  model: string
  providers: string[]
  has: string[]
  finalProvider: string | null
  failed: boolean
  servedModel: string | null
  status: number | null
  durationMs: number | null
  ttftMs: number | null
  completed: boolean
  inferenceRegion: Record<string, unknown> | null
}
function count(value: unknown) {
  return value === null ? null : integer(value)
}
function log(value: unknown): PinLog {
  const row = object(value)
  return {
    requestId: text(row.requestId),
    attemptIndex: integer(row.attemptIndex),
    timeMs: integer(row.timeMs),
    accountId: text(row.accountId),
    model: text(row.model),
    providers: array(row.providers, text),
    has: array(row.has, text),
    finalProvider: nullable(row.finalProvider),
    failed: boolean(row.failed),
    servedModel: nullable(row.servedModel),
    status: count(row.status),
    durationMs: count(row.durationMs),
    ttftMs: count(row.ttftMs),
    completed: boolean(row.completed),
    inferenceRegion: row.inferenceRegion === null ? null : object(row.inferenceRegion),
  }
}
export function getPinDetail(row: PinLog) {
  return request('POST', 'api/upstream-pin/logs/detail', { requestId: row.requestId, attemptIndex: row.attemptIndex }, (value) => {
    const data = object(value)
    return { entry: log(data.entry), response: object(data.response), message: nullable(data.message), limited: boolean(data.limited) }
  })
}
export function getPinLogs(offset: number) {
  return request('POST', 'api/upstream-pin/logs', { offset }, (value) => {
    const page = object(value)
    return {
      total: integer(page.total),
      entries: array(page.entries, log),
    }
  })
}
export const clearPinLogs = () => request('POST', 'api/upstream-pin/logs/clear', {}, value => boolean(object(value).cleared))
