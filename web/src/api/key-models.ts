import { array, boolean, object, request, text, version } from './index'

export interface KeyModelPolicy {
  enabled: boolean
  models: string[]
}

function policy(value: unknown) {
  const result = object(value)
  const data = object(result.value)
  return {
    version: version(result.version),
    value: { enabled: boolean(data.enabled), models: array(data.models, text) },
  }
}

export const getKeyModels = (clientKeyId: string) =>
  request('POST', 'api/key-models/get', { clientKeyId }, policy)

export const saveKeyModels = (
  clientKeyId: string,
  expectedVersion: number | null,
  value: KeyModelPolicy,
) => request('POST', 'api/key-models/save', { clientKeyId, expectedVersion, value }, policy)
