<script setup lang="ts">
import type { SelectOption, TableColumn } from '@codex-proxy/ui'
import type { ClineConfiguration } from '../api/cline'
import type { QuotaSource } from '../api/upstream'
import type { PinLog, PinRule } from '../api/upstream-pin'
import { ZButton, ZDialog, ZIconButton, ZInput, ZPopover, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { message } from '../api'
import { getCline, getClineModels, prepareClineCatalog, saveCline } from '../api/cline'
import { getQuotaSources } from '../api/upstream'
import { clearPinLogs, deletePin, getPinDetail, getPinLogs, getPins, savePin } from '../api/upstream-pin'
import { copyText } from '../clipboard'
import TablePagination from '../components/TablePagination.vue'
import { formatDateTime } from '../dates'

const props = defineProps<{ active: boolean }>()
const conversion = reactive({
  version: null as number | null,
  accountIds: [] as string[],
  activeAccountIds: [] as string[],
  pendingConfiguration: false,
  catalogEnabled: false,
  activeCatalogCount: 0,
  preparedConfiguration: '',
  selected: '',
  busy: false,
  error: '',
  removeId: '',
})
const state = reactive({
  rules: [] as PinRule[],
  accounts: [] as QuotaSource[],
  selectedAccountId: '',
  version: null as number | null,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
})
const editor = reactive({
  open: false,
  existing: false,
  originalModel: '',
  accountId: '',
  model: '',
  modelQuery: '',
  enabled: true,
  providers: '',
  has: '',
  busy: false,
  error: '',
})
const removal = reactive({ open: false, rule: null as PinRule | null, error: '' })
const catalog = reactive({ models: null as string[] | null, busy: false, error: '' })
const catalogPending = computed(() => conversion.catalogEnabled !== (conversion.activeCatalogCount > 0)
  || (conversion.catalogEnabled && (conversion.pendingConfiguration || !!conversion.preparedConfiguration)))
const modelOptions = computed(() => {
  const query = editor.modelQuery.trim()
  const search = query.toLowerCase()
  const ids = catalog.models ?? []
  const options: SelectOption[] = ids.filter(id => id.toLowerCase().includes(search))
    .map(id => ({ value: id, label: id }))
  if (query && !ids.includes(query))
    options.push({ value: query, label: query, description: '使用此名称' })
  return options
})
const logs = reactive({ entries: [] as PinLog[], offset: 0, total: 0, busy: false, error: '' })
const detail = reactive({ open: false, busy: false, error: '', value: null as Awaited<ReturnType<typeof getPinDetail>> | null })
const officialRule = computed(() => ['cline-pass/deepseek-v4.1-flash-1017', 'deepseek/deepseek-v4.1-flash-1017'].includes(editor.model.trim())
  && split(editor.providers).length === 1 && split(editor.providers)[0]?.toLowerCase() === 'deepseek')
const accountOptions = computed(() => state.accounts.map(account => ({ value: account.id, label: `${account.name}${account.enabled ? '' : '（已停用）'}` })))
const conversionRows = computed(() => [...new Set([...conversion.accountIds, ...conversion.activeAccountIds])].map(id => ({
  id,
  saved: conversion.accountIds.includes(id),
  registered: conversion.activeAccountIds.includes(id),
})))
const conversionOptions = computed(() => accountOptions.value.filter(account => !conversion.accountIds.includes(account.value)))
const conversionColumns: TableColumn<{ id: string, saved: boolean, registered: boolean }>[] = [
  { key: 'account', label: 'Cline 账号', kind: 'custom', size: 'xl' },
  { key: 'status', label: '转换状态', kind: 'custom', size: 'xl' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'lg' },
]
const columns: TableColumn<PinRule>[] = [
  { key: 'account', label: '上游账号', kind: 'custom', size: 'lg' },
  { key: 'model', label: '请求模型名称', kind: 'mono', size: '3xl' },
  { key: 'routing', label: '提供方 / 能力', kind: 'custom', size: 'xl' },
  { key: 'enabled', label: '状态', kind: 'custom', size: 'sm' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'lg' },
]
const logColumns: TableColumn<PinLog>[] = [
  { key: 'time', label: '时间', kind: 'custom', size: 'xl' },
  { key: 'target', label: '账号 / 请求模型名称', kind: 'custom', size: '3xl' },
  { key: 'routing', label: '已应用参数', kind: 'custom', size: 'xl' },
  { key: 'result', label: '上游来源信息', kind: 'custom', size: 'lg' },
  { key: 'timing', label: '首字 / 耗时', kind: 'custom', size: 'lg' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'sm' },
]
function elapsed(value: number | null) {
  return value === null ? '未提供' : `${(value / 1000).toFixed(2)} 秒`
}
const detailUsage = computed(() => {
  const usage = detail.value?.response.usage as Record<string, unknown> | undefined
  const input = usage?.prompt_tokens ?? usage?.input_tokens
  const output = usage?.completion_tokens ?? usage?.output_tokens
  const promptDetails = (usage?.prompt_tokens_details ?? usage?.input_tokens_details) as Record<string, unknown> | undefined
  const cached = promptDetails?.cached_tokens ?? usage?.prompt_cache_hit_tokens
  const elapsedMs = detail.value?.entry.durationMs
  const outputMs = elapsedMs === null || elapsedMs === undefined ? null : elapsedMs - (detail.value?.entry.ttftMs ?? 0)
  return {
    input: typeof input === 'number' ? input : '未提供',
    output: typeof output === 'number' ? output : '未提供',
    cached: typeof cached === 'number' ? cached : '未提供',
    rate: typeof cached === 'number' && typeof input === 'number' && input > 0 ? `${(cached / input * 100).toFixed(1)}%` : '未提供',
    speed: typeof output === 'number' && outputMs !== null && outputMs > 0 ? `${(output * 1000 / outputMs).toFixed(1)} Token/秒` : '未提供',
  }
})
async function showDetail(row: PinLog) {
  detail.open = true
  detail.busy = true
  detail.error = ''
  detail.value = null
  try {
    detail.value = await getPinDetail(row)
  }
  catch (error) {
    detail.error = message(error)
  }
  finally {
    detail.busy = false
  }
}
function name(id: string) {
  return state.accounts.find(account => account.id === id)?.name ?? id
}
function split(value: string) {
  return value.split(/[,，]/).map(value => value.trim()).filter(Boolean)
}
function sourceMatches(row: PinLog) {
  return row.finalProvider !== null && row.providers.includes(row.finalProvider)
}
function apply(value: { version: number | null, rules: PinRule[] }) {
  state.version = value.version
  state.rules = value.rules
}
function applyConversion(value: ClineConfiguration) {
  conversion.version = value.version
  conversion.accountIds = value.accountIds
  conversion.activeAccountIds = value.activeAccountIds
  conversion.activeCatalogCount = value.catalogModels.length
  conversion.pendingConfiguration = value.pendingConfiguration
  conversion.preparedConfiguration = ''
}
async function changeConversion(removeId?: string) {
  conversion.busy = true
  conversion.error = ''
  try {
    const ids = removeId
      ? conversion.accountIds.filter(id => id !== removeId)
      : [...conversion.accountIds, conversion.selected]
    applyConversion(await saveCline(conversion.version, ids))
    if (!ids.length)
      conversion.catalogEnabled = false
    conversion.selected = ''
    conversion.removeId = ''
    state.notice = conversion.pendingConfiguration ? '配置待应用' : '账号选择已保存'
  }
  catch (error) {
    conversion.error = message(error)
  }
  finally {
    conversion.busy = false
  }
}
async function copyConfiguration() {
  conversion.error = ''
  if (conversion.selected) {
    conversion.error = '请先点击加入转换配置，再复制'
    return
  }
  if (conversion.catalogEnabled && !conversion.preparedConfiguration) {
    await prepareCatalog()
    return
  }
  conversion.busy = true
  try {
    await copyText(conversion.catalogEnabled
      ? conversion.preparedConfiguration
      : JSON.stringify({ clineAccountIds: conversion.accountIds }, null, 2))
    state.notice = '实例配置已复制'
  }
  catch (error) {
    conversion.error = message(error)
  }
  finally {
    conversion.busy = false
  }
}
async function prepareCatalog() {
  conversion.busy = true
  conversion.error = ''
  try {
    const config = await prepareClineCatalog(conversion.accountIds)
    conversion.preparedConfiguration = JSON.stringify(config)
  }
  catch (error) {
    conversion.error = message(error)
  }
  finally {
    conversion.busy = false
  }
}
function changeCatalog(enabled: boolean) {
  conversion.catalogEnabled = enabled
  conversion.preparedConfiguration = ''
  conversion.error = ''
  state.notice = ''
  if (enabled && !conversion.selected)
    void prepareCatalog()
}
async function load() {
  if (state.busy || conversion.busy)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    const [config, accounts, cline] = await Promise.all([getPins(), getQuotaSources(), getCline()])
    apply(config)
    applyConversion(cline)
    conversion.catalogEnabled = cline.catalogModels.length > 0
    state.accounts = accounts
    if (!accounts.some(account => account.id === state.selectedAccountId))
      state.selectedAccountId = accounts.find(account => account.enabled)?.id ?? accounts[0]?.id ?? ''
    state.loaded = true
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}
async function loadCatalog() {
  if (catalog.busy || catalog.models !== null)
    return
  catalog.busy = true
  catalog.error = ''
  try {
    catalog.models = await getClineModels()
  }
  catch (error) {
    catalog.error = message(error)
  }
  finally {
    catalog.busy = false
  }
}
function edit(rule?: PinRule) {
  editor.existing = !!rule
  editor.originalModel = rule?.model ?? ''
  editor.accountId = rule?.accountId ?? state.selectedAccountId
  editor.model = rule?.model ?? ''
  editor.modelQuery = ''
  editor.providers = rule?.providers.join(', ') ?? 'deepseek'
  editor.has = rule?.has.join(', ') ?? 'vision, implicit-caching'
  editor.enabled = rule?.enabled ?? true
  editor.error = ''
  editor.open = true
  void loadCatalog()
}
async function save() {
  editor.busy = true
  editor.error = ''
  try {
    const rule = {
      accountId: editor.accountId,
      model: editor.model.trim(),
      enabled: editor.enabled,
      providers: split(editor.providers),
      has: split(editor.has),
    }
    if (state.rules.some(value => value.accountId === rule.accountId && value.model === rule.model
      && (!editor.existing || value.model !== editor.originalModel))) {
      editor.error = '此账号和请求模型名称已有规则'
      return
    }
    apply(await savePin(state.version, rule, editor.existing ? editor.originalModel : undefined))
    editor.open = false
    state.notice = '钉选规则已保存，匹配账号和模型的新请求将应用参数'
  }
  catch (error) {
    editor.error = message(error)
  }
  finally {
    editor.busy = false
  }
}
function confirmRemoval(rule: PinRule) {
  removal.rule = rule
  removal.error = ''
  removal.open = true
}
async function remove() {
  if (!removal.rule)
    return
  state.busy = true
  removal.error = ''
  try {
    apply(await deletePin(state.version, removal.rule))
    removal.open = false
    state.notice = '规则已删除，后续请求不再注入此规则的参数'
  }
  catch (error) {
    removal.error = message(error)
  }
  finally {
    state.busy = false
  }
}
async function readLogs(offset = logs.offset) {
  if (logs.busy)
    return
  logs.busy = true
  logs.error = ''
  try {
    let result = await getPinLogs(offset)
    if (offset && offset >= result.total) {
      offset = 0
      result = await getPinLogs(0)
    }
    logs.entries = result.entries
    logs.total = result.total
    logs.offset = offset
  }
  catch (error) {
    logs.error = message(error)
  }
  finally {
    logs.busy = false
  }
}
async function clearLogs() {
  logs.busy = true
  logs.error = ''
  try {
    await clearPinLogs()
    logs.entries = []
    logs.total = 0
    logs.offset = 0
  }
  catch (error) {
    logs.error = message(error)
  }
  finally {
    logs.busy = false
  }
}
watch(() => props.active, (active) => {
  if (active && !state.loaded)
    void load()
}, { immediate: true })
</script>

<template>
  <div class="panel">
    <section class="section">
      <div class="section-heading">
        <h2>协议转换</h2>
        <ZButton :loading="state.busy" :disabled="conversion.busy" @click="load">重新读取</ZButton>
      </div>
      <div class="conversion-setup">
        <div class="section-body">
          <div class="filter-bar">
            <label class="field"><span>1 · 选择转换账号</span><ZSelect :model-value="conversion.selected" :options="conversionOptions" filterable :disabled="!state.loaded || state.busy || conversion.busy" placeholder="选择已有 Cline 账号" @update:model-value="conversion.selected = $event ?? ''" /></label>
            <ZButton type="primary" :loading="conversion.busy" :disabled="!state.loaded || state.busy || !conversion.selected" @click="changeConversion()">加入转换配置</ZButton>
          </div>
        </div>
        <div class="conversion-apply">
          <h3>2 · 模型列表替换</h3>
          <div class="toolbar catalog-controls">
            <ZSwitch :model-value="conversion.catalogEnabled" :disabled="!state.loaded || state.busy || conversion.busy || !conversion.accountIds.length" label="替换下游模型列表" @update:model-value="changeCatalog" />
            <ZPopover trigger="hover-click" placement="bottom-start">
              <template #reference="{ id, open }">
                <ZIconButton size="small" aria-label="模型列表替换说明" :aria-describedby="open ? id : undefined">
                  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
                    <circle cx="12" cy="12" r="9" />
                    <path d="M12 11v5m0-8h.01" />
                  </svg>
                </ZIconButton>
              </template>
              <p class="catalog-tooltip">隐藏转换账号的原模型和 CPR 映射后的别名，换成 ClinePass 官方目录。其他账号的同名模型也会隐藏，已有模型白名单仍然有效</p>
            </ZPopover>
          </div>
          <p v-if="!catalogPending && conversion.activeCatalogCount" class="field-help">当前配置含 {{ conversion.activeCatalogCount }} 个 ClinePass 模型</p>
        </div>
        <div class="conversion-apply">
          <h3>3 · 应用实例配置</h3>
          <ZButton :loading="conversion.busy && conversion.catalogEnabled" :disabled="!state.loaded || state.busy || conversion.busy" @click="copyConfiguration">
            {{ conversion.catalogEnabled && !conversion.preparedConfiguration ? '生成目录配置' : '复制实例配置' }}
          </ZButton>
          <p v-if="catalogPending || conversion.pendingConfiguration" class="field-help" role="status">待保存到插件设置</p>
        </div>
      </div>
      <details class="conversion-guide">
        <summary>配置步骤</summary>
        <ol>
          <li>选择 Cline 账号，点击「加入转换配置」</li>
          <li>需要替换下游列表时打开开关；只做协议转换时保持关闭</li>
          <li>点击「复制实例配置」。如果看到「生成目录配置」，先生成再复制</li>
          <li>到「插件管理」，打开本插件卡片的管理窗口，再点「设置」。在「插件参数」用复制的 JSON 替换原内容，点击「保存设置」</li>
          <li>开启列表替换后，在客户端刷新模型列表，检查是否出现 <code>cline-pass/</code> 模型</li>
        </ol>
        <p>新增或移除转换账号时，保存前先停用相关账号并等请求结束，保存后再恢复</p>
        <p>首次设置绑定请参照发行包的 <code>INSTALL.md</code></p>
      </details>
      <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
      <p v-if="conversion.error && !conversion.removeId" class="error" role="alert">{{ conversion.error }}</p>
      <p v-if="state.notice" class="muted" role="status">{{ state.notice }}</p>
      <ZTable size="small" :columns="conversionColumns" :data="conversionRows" :loading="state.busy || conversion.busy" empty-text="暂无转换账号">
        <template #empty><p class="table-empty">暂无转换账号</p></template>
        <template #account="{ row }">{{ name(row.id) }}</template>
        <template #status="{ row }">{{ row.saved && row.registered ? '当前实例已配置' : row.saved ? '待应用实例配置' : '待应用后移除' }}</template>
        <template #actions="{ row }"><ZButton v-if="row.saved" size="small" variant="text" type="danger" :disabled="state.busy || conversion.busy" @click="conversion.removeId = row.id; conversion.error = ''">移除转换</ZButton></template>
      </ZTable>
    </section>
    <section class="section">
      <h2>DeepSeek 钉选</h2>
      <div class="filter-bar">
        <label class="field"><span>上游账号</span><ZSelect :model-value="state.selectedAccountId" :options="accountOptions" filterable :disabled="!state.loaded || state.busy || editor.busy" placeholder="选择需要钉选的上游账号" @update:model-value="state.selectedAccountId = $event ?? ''" /></label>
        <ZButton type="primary" :disabled="!state.loaded || state.busy || !state.accounts.length" @click="edit()">添加规则</ZButton>
      </div>
      <p class="field-help">Cline 原生账号先完成协议转换，Responses 反代账号可单独钉选，需启用「出站请求准备」</p>
      <p class="field-help">1017 模型仅允许 deepseek 时自动附加区域筛选，实际来源与响应模型以日志为准</p>
      <ZTable size="small" :columns="columns" :data="state.rules" :loading="state.busy" empty-text="暂无规则">
        <template #empty><p class="table-empty">暂无规则</p></template>
        <template #account="{ row }">{{ name(row.accountId) }}</template>
        <template #routing="{ row }"><div>{{ row.providers.join(', ') }}</div><div v-if="row.has.length" class="muted">{{ row.has.join(', ') }}</div></template>
        <template #enabled="{ row }">{{ row.enabled ? '已启用' : '已停用' }}</template>
        <template #actions="{ row }">
          <ZButton size="small" variant="text" :disabled="state.busy" @click="edit(row)">编辑</ZButton>
          <ZButton size="small" variant="text" type="danger" :disabled="state.busy" @click="confirmRemoval(row)">删除</ZButton>
        </template>
      </ZTable>
    </section>
    <section class="section">
      <div class="section-heading">
        <h2>转换与钉选日志</h2>
        <div class="toolbar">
          <ZButton :loading="logs.busy" @click="readLogs()">刷新日志</ZButton>
          <ZButton variant="text" :disabled="logs.busy || !logs.total" @click="clearLogs">清空日志</ZButton>
        </div>
      </div>
      <p v-if="logs.error" class="error" role="alert">{{ logs.error }}</p>
      <ZTable size="small" :columns="logColumns" :data="logs.entries" :loading="logs.busy" empty-text="暂无钉选记录">
        <template #empty><p class="table-empty">暂无钉选记录</p></template>
        <template #time="{ row }">{{ formatDateTime(row.timeMs) }}</template>
        <template #target="{ row }"><div>{{ name(row.accountId) }}</div><code>{{ row.model }}</code></template>
        <template #routing="{ row }"><div>{{ row.providers.join(', ') || '未命中' }}</div><div v-if="row.providers.length && row.has.length" class="muted">{{ row.has.join(', ') }}</div></template>
        <template #result="{ row }">
          <span :class="{ 'status-success': sourceMatches(row) }" :aria-label="sourceMatches(row) ? `${row.finalProvider}，与钉选一致` : undefined">{{ row.finalProvider ?? '未知' }}</span>
          <div v-if="row.providers.length && row.finalProvider && !sourceMatches(row)">来源不符合钉选</div>
          <div v-if="row.failed" class="muted">准备或调用失败</div>
          <div class="mono response-model">{{ row.servedModel ?? '响应模型未报告' }}</div>
        </template>
        <template #timing="{ row }"><div>{{ elapsed(row.ttftMs) }}</div><div class="muted">{{ row.completed ? elapsed(row.durationMs) : '处理中' }}</div></template>
        <template #actions="{ row }"><ZButton size="small" variant="text" @click="showDetail(row)">详情</ZButton></template>
      </ZTable>
      <TablePagination :offset="logs.offset" :total="logs.total" :page-size="25" :busy="logs.busy" @change="readLogs" />
    </section>
    <ZDialog v-model="editor.open" :title="editor.existing ? '编辑规则' : '添加规则'" width="44rem" :close-on-click-modal="!editor.busy" :close-on-press-escape="!editor.busy" :show-close="!editor.busy">
      <div class="panel">
        <div class="form-grid">
          <label class="field"><span>上游账号</span><ZSelect :model-value="editor.accountId" :options="accountOptions" filterable :disabled="editor.existing || editor.busy" placeholder="选择 Cline 或其反代账号" @update:model-value="editor.accountId = $event ?? ''" /></label>
          <label class="field">
            <span>请求模型名称</span>
            <ZSelect
              :model-value="editor.model || undefined"
              :options="modelOptions"
              :disabled="editor.busy"
              :empty-text="catalog.busy ? '正在读取 ClinePass 模型' : '输入完整名称以手动添加'"
              filterable remote
              placeholder="选择 ClinePass 模型或输入完整名称"
              @update:model-value="editor.model = $event ?? ''"
              @query-change="editor.modelQuery = $event"
              @visible-change="!$event && (editor.modelQuery = '')"
            >
              <template #option="{ option }">
                <span class="model-option">
                  <span v-if="option.description">{{ option.description }}：</span><code class="mono">{{ option.label }}</code>
                </span>
              </template>
            </ZSelect>
            <small class="field-help">按映射后的完整名称匹配，不会修改请求中的模型</small>
            <small v-if="catalog.error" class="field-help" role="status">目录读取失败，可手动输入：{{ catalog.error }}</small>
          </label>
          <label class="field"><span>允许的提供方</span><ZInput v-model="editor.providers" :disabled="editor.busy" placeholder="deepseek，多项用逗号分隔" /></label>
          <label class="field"><span>能力过滤</span><ZInput v-model="editor.has" :disabled="editor.busy" placeholder="vision, implicit-caching，留空关闭" /></label>
        </div>
        <div class="toolbar">
          <ZSwitch v-model="editor.enabled" :disabled="editor.busy" label="启用规则" />
        </div>
        <p v-if="officialRule" class="field-help">自动添加 inferenceRegion，筛除私有通道；能力留空不会关闭区域筛选，停用规则或修改模型、提供方可退出</p>
        <p v-if="editor.error" class="error" role="alert">{{ editor.error }}</p>
      </div>
      <template #footer>
        <ZButton :disabled="editor.busy" @click="editor.open = false">取消</ZButton>
        <ZButton type="primary" :loading="editor.busy" :disabled="!editor.accountId || !editor.model.trim() || !split(editor.providers).length" @click="save">保存规则</ZButton>
      </template>
    </ZDialog>
    <ZDialog v-model="detail.open" title="上游返回详情" width="52rem">
      <div class="panel">
        <p v-if="detail.busy" class="muted" role="status">正在读取详情</p>
        <p v-if="detail.error" class="error" role="alert">{{ detail.error }}</p>
        <template v-if="detail.value">
          <dl>
            <dt>上游账号</dt><dd>{{ name(detail.value.entry.accountId) }}</dd>
            <dt>请求模型名称</dt><dd class="mono">{{ detail.value.entry.model }}</dd>
            <dt>响应模型名称</dt><dd class="mono response-model">{{ detail.value.entry.servedModel ?? '未报告' }}</dd>
            <dt>上游来源信息</dt><dd :class="{ 'status-success': sourceMatches(detail.value.entry) }">{{ detail.value.entry.finalProvider ?? '未知' }}</dd>
            <dt>HTTP 状态</dt><dd>{{ detail.value.entry.status ?? '未提供' }}</dd>
            <dt>请求结果</dt><dd>{{ detail.value.entry.failed ? '失败' : detail.value.entry.completed ? '已完成' : '处理中' }}</dd>
            <dt>首字 / 耗时</dt><dd>{{ elapsed(detail.value.entry.ttftMs) }} / {{ elapsed(detail.value.entry.durationMs) }}</dd>
            <dt>输入 / 输出 Token</dt><dd>{{ detailUsage.input }} / {{ detailUsage.output }}</dd>
            <dt>缓存 Token / 命中率</dt><dd>{{ detailUsage.cached }} / {{ detailUsage.rate }}</dd>
            <dt>输出速率</dt><dd>{{ detailUsage.speed }}</dd>
          </dl>
          <p v-if="detail.value.message" class="error">{{ detail.value.message }}</p>
          <h3>已应用路由参数</h3>
          <pre class="response-json">{{ JSON.stringify({ only: detail.value.entry.providers, has: detail.value.entry.has, inferenceRegion: detail.value.entry.inferenceRegion }, null, 2) }}</pre>
          <h3>上游响应诊断</h3>
          <p class="field-help">展示上游回报的用量、结束原因和路由诊断字段，不包含回答正文、工具参数或凭据</p>
          <pre class="response-json">{{ JSON.stringify(detail.value.response, null, 2) }}</pre>
          <p v-if="detail.value.limited" class="notice">部分诊断信息超过长度或容量限制，仅保留有界内容</p>
        </template>
      </div>
      <template #footer><ZButton @click="detail.open = false">关闭</ZButton></template>
    </ZDialog>
    <ZDialog v-model="removal.open" title="删除规则" :close-on-click-modal="!state.busy" :close-on-press-escape="!state.busy" :show-close="!state.busy">
      <p>删除 {{ removal.rule?.model }} 的钉选规则，后续请求恢复原有路由参数</p>
      <p v-if="removal.error" class="error" role="alert">{{ removal.error }}</p>
      <template #footer><ZButton :disabled="state.busy" @click="removal.open = false">取消</ZButton><ZButton type="danger" :loading="state.busy" @click="remove">删除规则</ZButton></template>
    </ZDialog>
    <ZDialog :model-value="!!conversion.removeId" title="移除协议转换" :close-on-click-modal="!conversion.busy" :close-on-press-escape="!conversion.busy" :show-close="!conversion.busy" @update:model-value="!$event && (conversion.removeId = '')">
      <p>从选择中移除 {{ name(conversion.removeId) }}，复制并应用新的实例配置后恢复原生 Responses 路径</p>
      <p class="notice">请先停用依赖转换的账号，确认原生协议可用后再恢复</p>
      <p v-if="conversion.error" class="error" role="alert">{{ conversion.error }}</p>
      <template #footer><ZButton :disabled="conversion.busy" @click="conversion.removeId = ''">取消</ZButton><ZButton type="danger" :loading="conversion.busy" @click="changeConversion(conversion.removeId)">移除转换</ZButton></template>
    </ZDialog>
  </div>
</template>

<style scoped>
.form-grid { align-items: start; }
.model-option { min-width: 0; text-align: left; white-space: normal; }
.response-model { overflow-wrap: anywhere; font-weight: 600; }
.response-json { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--z-font-size-sm); }
.conversion-setup {
  display: grid;
  grid-template-columns: minmax(0, 1.15fr) repeat(2, minmax(0, 1fr));
  gap: 24px;
  align-items: start;
}
.conversion-apply {
  display: grid;
  justify-items: start;
  gap: 12px;
  min-width: 0;
  padding-left: 24px;
  border-left: 1px solid var(--z-color-border-secondary);
}
.catalog-controls { gap: 4px; }
.catalog-tooltip { width: min(18rem, 70vw); margin: 0; line-height: 1.5; }
.conversion-guide {
  color: var(--z-color-text-secondary);
}
.conversion-guide summary {
  width: fit-content;
  cursor: pointer;
}
.conversion-guide ol {
  display: grid;
  gap: 4px;
  margin: 8px 0 0;
  padding-inline-start: 1.5em;
  list-style: decimal;
}
.conversion-guide p {
  margin: 8px 0 0;
}
@container toolkit (max-width: 1040px) {
  .conversion-setup {
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
  }
  .conversion-apply {
    padding: 16px 0 0;
    border-left: 0;
    border-top: 1px solid var(--z-color-border-secondary);
  }
}
</style>
