import { array, boolean, integer, nullable, object, request, text, version } from './index'

export interface PriorityAccount {
  id: string
  provider: string
  name: string
  enabled: boolean
}

export interface PriorityRule {
  provider: string
  model: string
  enabled: boolean
  accountIds: string[]
}

function account(value: unknown): PriorityAccount {
  const row = object(value)
  return {
    id: text(row.id),
    provider: text(row.provider),
    name: text(row.name),
    enabled: boolean(row.enabled),
  }
}

function rule(value: unknown): PriorityRule {
  const row = object(value)
  return {
    provider: text(row.provider),
    model: text(row.model),
    enabled: boolean(row.enabled),
    accountIds: array(row.accountIds, text),
  }
}

function configuration(value: unknown) {
  const data = object(value)
  return { version: version(data.version), rules: array(data.rules, rule) }
}

export function getPriorities() {
  return request('GET', 'api/model-priority', undefined, value => ({
    ...configuration(value),
    accounts: array(object(value).accounts, account),
  }))
}

export function lookupPriorityAccounts(provider: string, model: string) {
  return request('POST', 'api/model-priority/lookup', { provider, model }, (value) => {
    const data = object(value)
    return {
      accounts: array(data.accounts, account),
      failures: array(data.failures, (value) => {
        const row = object(value)
        return { id: text(row.id), name: text(row.name), message: text(row.message) }
      }),
    }
  })
}

export function savePriority(expectedVersion: number | null, rule: PriorityRule) {
  return request('POST', 'api/model-priority/save', { expectedVersion, rule }, configuration)
}

export function deletePriority(expectedVersion: number | null, provider: string, model: string) {
  return request('POST', 'api/model-priority/delete', { expectedVersion, provider, model }, configuration)
}

export interface PriorityDecision {
  attemptIndex: number
  kind: string
  primaryAccountId: string | null
  selectedAccountId: string | null
}

export interface PriorityLog {
  requestId: string
  timeMs: number
  provider: string
  model: string | null
  decisions: PriorityDecision[]
  decisionsTruncated: boolean
  completed: boolean
  actualAccountId: string | null
  outcome: string | null
  sendState: string | null
  attemptCount: number | null
}

export function getPriorityLogs(offset: number) {
  return request('POST', 'api/model-priority/logs', { offset }, (value) => {
    const data = object(value)
    return {
      entries: array(data.entries, (value) => {
        const row = object(value)
        return {
          requestId: text(row.requestId),
          timeMs: integer(row.timeMs),
          provider: text(row.provider),
          model: nullable(row.model),
          decisions: array(row.decisions, (value) => {
            const decision = object(value)
            return { attemptIndex: integer(decision.attemptIndex), kind: text(decision.kind), primaryAccountId: nullable(decision.primaryAccountId), selectedAccountId: nullable(decision.selectedAccountId) }
          }),
          decisionsTruncated: boolean(row.decisionsTruncated),
          completed: boolean(row.completed),
          actualAccountId: nullable(row.actualAccountId),
          outcome: nullable(row.outcome),
          sendState: nullable(row.sendState),
          attemptCount: row.attemptCount === null ? null : integer(row.attemptCount),
        }
      }),
      total: integer(data.total),
      capacity: integer(data.capacity),
      pageSize: integer(data.pageSize),
    }
  })
}

export const clearPriorityLogs = () => request('POST', 'api/model-priority/logs/clear', {}, value => boolean(object(value).cleared))
