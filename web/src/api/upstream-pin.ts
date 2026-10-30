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
export const savePin = (expectedVersion: number | null, rule: PinRule, originalModel?: string) =>
  request('POST', 'api/upstream-pin/save', { expectedVersion, rule, originalModel }, configuration)
export const deletePin = (expectedVersion: number | null, rule: PinRule) =>
  request('POST', 'api/upstream-pin/delete', { expectedVersion, accountId: rule.accountId, model: rule.model }, configuration)

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
}
export function getPinLogs(offset: number) {
  return request('POST', 'api/upstream-pin/logs', { offset }, (value) => {
    const page = object(value)
    return {
      total: integer(page.total),
      entries: array(page.entries, (value): PinLog => {
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
        }
      }),
    }
  })
}
export const clearPinLogs = () => request('POST', 'api/upstream-pin/logs/clear', {}, value => boolean(object(value).cleared))
