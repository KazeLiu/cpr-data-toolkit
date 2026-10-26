import { array, boolean, object, request, text, version } from './index'

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

export const getPriorities = () =>
  request('GET', 'api/model-priority', undefined, (value) => ({
    ...configuration(value),
    accounts: array(object(value).accounts, account),
  }))

export const lookupPriorityAccounts = (provider: string, model: string) =>
  request('POST', 'api/model-priority/lookup', { provider, model }, (value) => {
    const data = object(value)
    return {
      accounts: array(data.accounts, account),
      failures: array(data.failures, (value) => {
        const row = object(value)
        return { id: text(row.id), name: text(row.name), message: text(row.message) }
      }),
    }
  })

export const savePriority = (expectedVersion: number | null, rule: PriorityRule) =>
  request('POST', 'api/model-priority/save', { expectedVersion, rule }, configuration)

export const deletePriority = (expectedVersion: number | null, provider: string, model: string) =>
  request('POST', 'api/model-priority/delete', { expectedVersion, provider, model }, configuration)
