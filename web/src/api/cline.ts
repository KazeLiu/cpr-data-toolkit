import { array, boolean, object, request, text, version } from './index'

export interface ClineConfiguration {
  version: number | null
  accountIds: string[]
  activeAccountIds: string[]
  catalogModels: string[]
  pendingConfiguration: boolean
}
function configuration(value: unknown): ClineConfiguration {
  const data = object(value)
  return {
    version: version(data.version),
    accountIds: array(data.accountIds, text),
    activeAccountIds: array(data.activeAccountIds, text),
    catalogModels: array(data.catalogModels, text),
    pendingConfiguration: boolean(data.pendingConfiguration),
  }
}
export const getCline = () => request('GET', 'api/cline', null, configuration)
export const getClineModels = () => request('GET', 'api/cline/models', null, value => array(object(value).models, text))
export const prepareClineCatalog = (accountIds: string[]) =>
  request('POST', 'api/cline/catalog/prepare', { accountIds }, object)
export const saveCline = (expectedVersion: number | null, accountIds: string[]) =>
  request('POST', 'api/cline/save', { expectedVersion, accountIds }, configuration)
