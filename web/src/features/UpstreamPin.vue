<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { QuotaSource } from '../api/upstream'
import type { PinLog, PinRule } from '../api/upstream-pin'
import { ZButton, ZDialog, ZInput, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { message } from '../api'
import { getQuotaSources } from '../api/upstream'
import { clearPinLogs, deletePin, getPinLogs, getPins, savePin } from '../api/upstream-pin'

const props = defineProps<{ active: boolean }>()
const presetModel = 'cline-pass/deepseek-v4.1-flash-1017'
const state = reactive({
  rules: [] as PinRule[],
  accounts: [] as QuotaSource[],
  version: null as number | null,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
})
const editor = reactive({
  open: false,
  existing: false,
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
const accountOptions = computed(() => state.accounts.map(account => ({ value: account.id, label: `${account.name}${account.enabled ? '' : '（已停用）'}` })))
const columns: TableColumn<PinRule>[] = [
  { key: 'account', label: '上游账号', kind: 'custom', size: 'lg' },
  { key: 'model', label: '最终上游模型 ID', kind: 'mono', size: '3xl' },
  { key: 'routing', label: '提供方 / 能力', kind: 'custom', size: 'xl' },
  { key: 'enabled', label: '状态', kind: 'custom', size: 'sm' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'lg' },
]
const logColumns: TableColumn<PinLog>[] = [
  { key: 'time', label: '时间 / 请求', kind: 'custom', size: 'xl' },
  { key: 'target', label: '账号 / 模型', kind: 'custom', size: '3xl' },
  { key: 'routing', label: '已应用参数', kind: 'custom', size: 'xl' },
  { key: 'result', label: '上游报告', kind: 'custom', size: 'lg' },
]
function name(id: string) {
  return state.accounts.find(account => account.id === id)?.name ?? id
}
function split(value: string) {
  return value.split(/[,，]/).map(value => value.trim()).filter(Boolean)
}
function apply(value: { version: number | null, rules: PinRule[] }) {
  state.version = value.version
  state.rules = value.rules
}
async function load() {
  if (state.busy)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    const [config, accounts] = await Promise.all([getPins(), getQuotaSources()])
    apply(config)
    state.accounts = accounts
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
  editor.accountId = rule?.accountId ?? state.accounts.find(account => account.enabled)?.id ?? state.accounts[0]?.id ?? ''
  editor.model = rule?.model ?? presetModel
  editor.providers = rule?.providers.join(', ') ?? 'deepseek'
  editor.has = rule?.has.join(', ') ?? 'vision, implicit-caching'
  editor.enabled = rule?.enabled ?? true
  editor.error = ''
  editor.open = true
}
function preset() {
  if (!editor.existing)
    editor.model = presetModel
  editor.providers = 'deepseek'
  editor.has = 'vision, implicit-caching'
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
    if (!editor.existing && state.rules.some(value => value.accountId === rule.accountId && value.model === rule.model)) {
      editor.error = '此账号和模型已有规则，请在列表中编辑'
      return
    }
    apply(await savePin(state.version, rule))
    editor.open = false
    state.notice = '规则已保存，匹配 outbound 绑定的新请求将应用参数'
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
      <div class="toolbar">
        <ZButton type="primary" :disabled="!state.loaded || state.busy || !state.accounts.length" @click="edit()">新增规则</ZButton>
        <ZButton :loading="state.busy" @click="load">重新读取</ZButton>
      </div>
      <p class="notice">在宿主启用本插件的 outbound 绑定，并选择需要生效的 Key 或账号范围</p>
      <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
      <p v-if="state.notice" class="muted" role="status">{{ state.notice }}</p>
      <ZTable :columns="columns" :data="state.rules" :loading="state.busy" empty-text="暂无规则，新增后可限制 Cline 网关的模型提供方">
        <template #account="{ row }">{{ name(row.accountId) }}</template>
        <template #routing="{ row }"><div>{{ row.providers.join(', ') }}</div><div class="muted">{{ row.has.join(', ') || '不使用能力过滤' }}</div></template>
        <template #enabled="{ row }">{{ row.enabled ? '已启用' : '已停用' }}</template>
        <template #actions="{ row }">
          <ZButton size="small" variant="text" :disabled="state.busy" @click="edit(row)">编辑</ZButton>
          <ZButton size="small" variant="text" type="danger" :disabled="state.busy" @click="confirmRemoval(row)">删除</ZButton>
        </template>
      </ZTable>
      <p class="muted">规则按账号和发送前的模型 ID 精确匹配，仅处理 Responses，不改变 CPR 的账号选择或模型映射</p>
    </section>
    <section class="section">
      <div class="toolbar">
        <h3>钉选日志</h3>
        <ZButton :loading="logs.busy" @click="readLogs()">刷新日志</ZButton>
        <ZButton :disabled="logs.busy || !logs.total" @click="clearLogs">清空日志</ZButton>
      </div>
      <p class="notice">查看最终提供方需同时启用 attempt 绑定，上游未回报或协议转换丢失元数据时显示未知</p>
      <p v-if="logs.error" class="error" role="alert">{{ logs.error }}</p>
      <ZTable :columns="logColumns" :data="logs.entries" :loading="logs.busy" empty-text="暂无钉选记录，发送请求后刷新日志">
        <template #time="{ row }"><div>{{ new Date(row.timeMs).toLocaleString() }}</div><div class="muted pin-id">{{ row.requestId }} · 第 {{ row.attemptIndex }} 次</div></template>
        <template #target="{ row }"><div>{{ name(row.accountId) }}</div><code>{{ row.model }}</code></template>
        <template #routing="{ row }"><div>{{ row.providers.join(', ') }}</div><div class="muted">{{ row.has.join(', ') || '无能力过滤' }}</div></template>
        <template #result="{ row }">
          <span :class="row.finalProvider && !row.providers.includes(row.finalProvider) ? 'error' : ''">{{ row.finalProvider ?? '未知' }}</span>
          <div v-if="row.finalProvider && !row.providers.includes(row.finalProvider)" class="error">来源不符合限制</div>
          <div v-if="row.failed" class="muted">准备或调用失败</div>
        </template>
      </ZTable>
      <div class="toolbar">
        <ZButton :disabled="logs.busy || logs.offset === 0" @click="readLogs(logs.offset - 25)">上一页</ZButton>
        <span class="muted">{{ logs.total ? logs.offset + 1 : 0 }}–{{ Math.min(logs.offset + 25, logs.total) }} / {{ logs.total }}</span>
        <ZButton :disabled="logs.busy || logs.offset + 25 >= logs.total" @click="readLogs(logs.offset + 25)">下一页</ZButton>
      </div>
    </section>
    <ZDialog v-model="editor.open" :title="editor.existing ? '编辑钉选规则' : '新增钉选规则'" width="44rem" :close-on-click-modal="!editor.busy" :close-on-press-escape="!editor.busy" :show-close="!editor.busy">
      <div class="panel">
        <div class="form-grid">
          <label class="field"><span>上游账号</span><ZSelect :model-value="editor.accountId" :options="accountOptions" filterable :disabled="editor.existing || editor.busy" placeholder="选择 Cline 或其反代账号" @update:model-value="editor.accountId = $event ?? ''" /></label>
          <label class="field"><span>最终上游模型 ID</span><ZInput v-model="editor.model" :disabled="editor.existing || editor.busy" placeholder="填写最终请求正文中的 model" /></label>
          <label class="field"><span>允许的提供方</span><ZInput v-model="editor.providers" :disabled="editor.busy" placeholder="deepseek，多项用逗号分隔" /></label>
          <label class="field"><span>能力过滤</span><ZInput v-model="editor.has" :disabled="editor.busy" placeholder="vision, implicit-caching，留空关闭" /></label>
        </div>
        <div class="toolbar">
          <ZButton :disabled="editor.busy" @click="preset">应用 DeepSeek 1017 预设</ZButton>
          <ZSwitch v-model="editor.enabled" :disabled="editor.busy" label="启用规则" />
        </div>
        <p class="notice">DeepSeek 预设使用 only 与 has 的组合，已有规则不会自动换模型，实际来源以日志中的上游回报为准</p>
        <p class="muted">上游需支持 Responses 及这些路由字段，本插件保留原协议和 URL</p>
        <p v-if="editor.error" class="error" role="alert">{{ editor.error }}</p>
      </div>
      <template #footer>
        <ZButton :disabled="editor.busy" @click="editor.open = false">取消</ZButton>
        <ZButton type="primary" :loading="editor.busy" :disabled="!editor.accountId || !editor.model.trim() || !split(editor.providers).length" @click="save">保存规则</ZButton>
      </template>
    </ZDialog>
    <ZDialog v-model="removal.open" title="删除钉选规则" :close-on-click-modal="!state.busy" :close-on-press-escape="!state.busy" :show-close="!state.busy">
      <p>删除 {{ removal.rule?.model }} 的钉选规则，后续请求恢复原有路由参数</p>
      <p v-if="removal.error" class="error" role="alert">{{ removal.error }}</p>
      <template #footer><ZButton :disabled="state.busy" @click="removal.open = false">取消</ZButton><ZButton type="danger" :loading="state.busy" @click="remove">删除规则</ZButton></template>
    </ZDialog>
  </div>
</template>

<style scoped>
.pin-id {
  overflow-wrap: anywhere;
}
</style>
