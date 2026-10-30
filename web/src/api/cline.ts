import { array, boolean, object, request, text, version } from './index'

export interface ClineConfiguration {
  version: number | null
  accountIds: string[]
  activeAccountIds: string[]
  pendingConfiguration: boolean
}
function configuration(value: unknown): ClineConfiguration {
  const data = object(value)
  return {
    version: version(data.version),
    accountIds: array(data.accountIds, text),
    activeAccountIds: array(data.activeAccountIds, text),
    pendingConfiguration: boolean(data.pendingConfiguration),
  }
}
export const getCline = () => request('GET', 'api/cline', null, configuration)
export const saveCline = (expectedVersion: number | null, accountIds: string[]) =>
  request('POST', 'api/cline/save', { expectedVersion, accountIds }, configuration)
