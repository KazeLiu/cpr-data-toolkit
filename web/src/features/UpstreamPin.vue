<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { ClineConfiguration } from '../api/cline'
import type { QuotaSource } from '../api/upstream'
import type { PinLog, PinRule } from '../api/upstream-pin'
import { ZButton, ZDialog, ZInput, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { message } from '../api'
import { getCline, saveCline } from '../api/cline'
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
  enabled: true,
  providers: '',
  has: '',
  busy: false,
  error: '',
})
const removal = reactive({ open: false, rule: null as PinRule | null, error: '' })
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
  conversion.pendingConfiguration = value.pendingConfiguration
}
async function changeConversion(removeId?: string) {
  conversion.busy = true
  conversion.error = ''
  try {
    const ids = removeId
      ? conversion.accountIds.filter(id => id !== removeId)
      : [...conversion.accountIds, conversion.selected]
    applyConversion(await saveCline(conversion.version, ids))
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
  try {
    await copyText(JSON.stringify({ clineAccountIds: conversion.accountIds }, null, 2))
    state.notice = '实例配置已复制'
  }
  catch (error) {
    conversion.error = message(error)
  }
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
function edit(rule?: PinRule) {
  editor.existing = !!rule
  editor.originalModel = rule?.model ?? ''
  editor.accountId = rule?.accountId ?? state.selectedAccountId
  editor.model = rule?.model ?? ''
  editor.providers = rule?.providers.join(', ') ?? 'deepseek'
  editor.has = rule?.has.join(', ') ?? 'vision, implicit-caching'
  editor.enabled = rule?.enabled ?? true
  editor.error = ''
  editor.open = true
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
          <p class="field-help">使用已在宿主创建、填入 Cline API密钥的 OpenAI 账号</p>
        </div>
        <div class="conversion-apply">
          <h3>2 · 应用实例配置</h3>
          <ZButton :disabled="!state.loaded || state.busy || conversion.busy" @click="copyConfiguration">复制实例配置</ZButton>
          <p class="field-help">插件管理 → 本插件卡片设置 → 弹窗内设置，粘贴并保存</p>
          <p class="field-help">启用「上游调用」，「插件失败时」选择「拒绝请求」</p>
          <p v-if="conversion.pendingConfiguration" class="notice" role="status">应用前先停用相关账号，等待请求结束，配置完成后再恢复</p>
        </div>
      </div>
      <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
      <p v-if="conversion.error && !conversion.removeId" class="error" role="alert">{{ conversion.error }}</p>
      <p v-if="state.notice" class="muted" role="status">{{ state.notice }}</p>
      <ZTable size="small" :columns="conversionColumns" :data="conversionRows" :loading="state.busy || conversion.busy" empty-text="暂无转换账号">
        <template #empty><p class="table-empty">暂无转换账号</p></template>
        <template #account="{ row }">{{ name(row.id) }}</template>
        <template #status="{ row }">{{ row.saved && row.registered ? '当前实例已配置' : row.saved ? '待应用实例配置' : '待应用后移除' }}</template>
        <template #actions="{ row }"><ZButton v-if="row.saved" size="small" variant="text" type="danger" :disabled="state.busy || conversion.busy" @click="conversion.removeId = row.id; conversion.error = ''">移除转换</ZButton></template>
      </ZTable>
      <div class="protocol-info">
        <h3>Responses → Cline Chat Completions</h3>
        <p class="field-help">固定发送至 <code>api.cline.bot/api/v1/chat/completions</code>，账号提供凭据和代理，模型映射与权限由宿主管理</p>
        <details>
          <summary>协议兼容范围</summary>
          <p>支持 HTTP 文本、图片 URL、函数工具和完整历史</p>
          <p>不支持 <code>previous_response_id</code>、WebSocket、加密推理历史或原生工具</p>
        </details>
      </div>
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
        <template #routing="{ row }"><div>{{ row.providers.join(', ') || '未命中' }}</div><div v-if="row.providers.length && row.has.length" class="muted">{{ row.has.join(', ') }}</div><div v-if="row.inferenceRegion" class="muted">已附加区域筛选</div></template>
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
            <ZInput v-model="editor.model" :disabled="editor.busy" placeholder="例如 cline-pass/deepseek-v4.1-flash-1017" />
            <small class="field-help">按完整名称精确匹配，不会修改请求中的模型<br>可从下方日志复制，有模型映射时填映射后的名称</small>
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
.response-model { overflow-wrap: anywhere; font-weight: 600; }
.response-json { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--z-font-size-sm); }
.conversion-setup {
  display: grid;
  grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
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
.conversion-apply > .notice {
  margin: 0;
}
.protocol-info {
  display: grid;
  gap: 8px;
  padding-top: 16px;
  border-top: 1px solid var(--z-color-border-secondary);
  overflow-wrap: anywhere;
}
.protocol-info summary {
  width: fit-content;
  color: var(--z-color-text-secondary);
  cursor: pointer;
}
.protocol-info details p {
  margin: 8px 0 0;
  color: var(--z-color-text-secondary);
}
@container toolkit (max-width: 900px) {
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
