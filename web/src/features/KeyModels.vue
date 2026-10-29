<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { Key } from '../api'
import { ZButton, ZCheckbox, ZDialog, ZInput, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { getKeys, getModels, message } from '../api'
import { getKeyModels, saveKeyModels } from '../api/key-models'

const props = defineProps<{ active: boolean }>()
const state = reactive({
  keys: [] as Key[],
  keysLoaded: false,
  key: '',
  version: null as number | null,
  enabled: false,
  models: [] as string[],
  catalog: [] as string[],
  catalogLoaded: false,
  catalogError: '',
  loaded: false,
  busy: false,
  error: '',
  notice: '',
  saved: '',
  search: '',
  selectedOnly: false,
  offset: 0,
})
const confirmation = reactive({ open: false, key: '' })
const blockAll = reactive({ open: false })
const snapshot = () => JSON.stringify({ enabled: state.enabled, models: [...state.models].sort() })
const dirty = computed(() => state.loaded && snapshot() !== state.saved)
const selected = computed(() => new Set(state.models))
const catalog = computed(() => new Set(state.catalog))
const keyOptions = computed(() => state.keys.map(key => ({
  value: key.id,
  label: `${key.name} · ${key.id}${key.enabled ? '' : '（已停用）'}`,
})))
const rows = computed(() => {
  const query = state.search.trim().toLowerCase()
  return [...new Set([...state.catalog, ...state.models])]
    .filter(id => (!state.selectedOnly || selected.value.has(id)) && id.toLowerCase().includes(query))
    .sort()
    .map(id => ({ id, available: catalog.value.has(id) }))
})
const pageRows = computed(() => rows.value.slice(state.offset, state.offset + 50))
const columns: TableColumn<{ id: string, available: boolean }>[] = [
  { key: 'allowed', label: '允许', size: 'xs', kind: 'custom' },
  { key: 'id', label: '下游模型 ID', size: '3xl', kind: 'mono' },
  { key: 'available', label: '目录状态', size: 'lg', kind: 'custom' },
]

function apply(result: Awaited<ReturnType<typeof getKeyModels>>) {
  state.version = result.version
  state.enabled = result.value.enabled
  state.models = result.value.models
  state.loaded = true
  state.saved = snapshot()
}
async function loadKeys() {
  state.busy = true
  state.error = ''
  try {
    state.keys = await getKeys()
    state.keysLoaded = true
  }
  catch (error) { state.error = message(error) }
  finally { state.busy = false }
}
async function readCatalog() {
  state.catalogError = ''
  try {
    state.catalog = await getModels(state.key)
    state.catalogLoaded = true
  }
  catch (error) { state.catalogError = `模型目录读取失败：${message(error)}，已保留白名单` }
}
async function load(key: string) {
  state.busy = true
  state.key = key
  state.loaded = false
  state.version = null
  state.models = []
  state.enabled = false
  state.catalog = []
  state.catalogLoaded = false
  state.search = ''
  state.selectedOnly = false
  state.offset = 0
  state.error = ''
  state.catalogError = ''
  state.notice = ''
  try {
    apply(await getKeyModels(key))
    await readCatalog()
  }
  catch (error) { state.error = message(error) }
  finally { state.busy = false }
}
function chooseKey(key: string) {
  if (!key || key === state.key) return
  if (dirty.value) {
    confirmation.key = key
    confirmation.open = true
  }
  else { void load(key) }
}
function reload() {
  if (dirty.value) {
    confirmation.key = state.key
    confirmation.open = true
  }
  else { void load(state.key) }
}
function discard() {
  confirmation.open = false
  void load(confirmation.key)
}
async function refreshCatalog() {
  state.busy = true
  try { await readCatalog() }
  finally { state.busy = false }
}
function toggle(id: string, checked: boolean) {
  state.models = checked ? [...state.models, id] : state.models.filter(model => model !== id)
  state.notice = ''
}
function selectPage() {
  const models = [...new Set([...state.models, ...pageRows.value.map(row => row.id)])]
  if (models.length > 1000) {
    state.error = '每个 Key 最多允许 1000 个模型'
    return
  }
  state.models = models
}
function confirmSave() {
  if (state.enabled && !state.models.length) blockAll.open = true
  else void save()
}
async function save() {
  blockAll.open = false
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    apply(await saveKeyModels(state.key, state.version, { enabled: state.enabled, models: state.models }))
    state.notice = state.enabled ? '白名单已保存，匹配 request 绑定的新请求使用此权限' : '已恢复宿主原有模型权限'
  }
  catch (error) { state.error = message(error) }
  finally { state.busy = false }
}
watch(() => [state.search, state.selectedOnly], () => { state.offset = 0 })
watch(() => rows.value.length, () => {
  if (state.offset >= rows.value.length) state.offset = 0
})
watch(() => props.active, (active) => {
  if (active && !state.keysLoaded && !state.busy) void loadKeys()
}, { immediate: true })
</script>

<template>
  <section class="panel">
    <p class="notice">在宿主启用本插件的 request 绑定，Key 范围留空或包含目标 Key，账号范围留空，失败策略选择拒绝请求</p>
    <div class="form-grid">
      <label class="field">
        <span>Client Key</span>
        <ZSelect :model-value="state.key" :options="keyOptions" filterable :disabled="state.busy" placeholder="选择要配置的 Key" @update:model-value="chooseKey($event ?? '')" />
      </label>
    </div>
    <div class="toolbar">
      <ZButton :loading="state.busy" :disabled="state.busy" @click="loadKeys">刷新 Key 列表</ZButton>
      <ZButton :disabled="!state.key || state.busy" @click="reload">重新读取权限</ZButton>
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <p v-if="!state.keys.length && state.keysLoaded" class="muted">暂无 Client Key，请先在宿主创建</p>
    <p v-else-if="!state.key" class="muted">选择 Key 后配置其下游模型权限</p>
    <template v-if="state.loaded">
      <div class="toolbar">
        <ZSwitch v-model="state.enabled" :disabled="state.busy" aria-label="限制模型" />
        <span>限制模型</span>
        <span class="muted">{{ state.enabled ? `仅允许已选的 ${state.models.length} 个模型` : '不限制，沿用宿主原有模型权限' }}</span>
      </div>
      <p class="muted">按下游模型 ID 精确匹配，区分大小写 · 白名单不会扩大 Key 的分组权限</p>
      <div class="toolbar">
        <ZButton type="primary" :loading="state.busy" :disabled="state.busy || !dirty" @click="confirmSave">保存权限</ZButton>
        <span v-if="dirty" class="muted">有未保存修改</span>
        <span v-if="state.notice && !dirty" class="success" role="status">{{ state.notice }}</span>
      </div>
      <section class="section">
        <div class="toolbar">
          <h2>模型白名单</h2>
          <ZButton :loading="state.busy" :disabled="state.busy" @click="refreshCatalog">获取模型列表</ZButton>
          <span class="muted">已选 {{ state.models.length }} / 1000</span>
        </div>
        <p v-if="!state.enabled" class="muted">限制关闭时可编辑白名单，保存后仍不限制模型</p>
        <p v-if="state.enabled && !state.models.length" class="error" role="status">白名单为空，保存后将禁止此 Key 请求任何模型</p>
        <p v-if="state.catalogError" class="error" role="alert">{{ state.catalogError }}</p>
        <div class="toolbar">
          <ZInput v-model="state.search" aria-label="搜索模型 ID" placeholder="搜索模型 ID" style="max-width: 360px" />
          <ZCheckbox v-model="state.selectedOnly" label="仅看已选" />
          <ZButton :disabled="state.busy || !pageRows.length" @click="selectPage">选择本页</ZButton>
          <ZButton :disabled="state.busy || !state.models.length" @click="state.models = []">清空选择</ZButton>
        </div>
        <ZTable :columns="columns" :data="pageRows" row-key="id" size="small" :loading="state.busy" :empty-text="state.catalogLoaded ? '没有匹配的模型' : '获取模型列表后选择允许的模型'">
          <template #allowed="{ row }">
            <ZCheckbox :model-value="selected.has(row.id)" :disabled="state.busy || (!selected.has(row.id) && state.models.length >= 1000)" :aria-label="`允许 ${row.id}`" @update:model-value="toggle(row.id, $event)" />
          </template>
          <template #available="{ row }">{{ row.available ? '当前目录可见' : state.catalogLoaded ? '当前目录未列出，保留选择' : '目录状态未确认' }}</template>
        </ZTable>
        <div class="toolbar">
          <ZButton :disabled="state.offset === 0" @click="state.offset -= 50">上一页</ZButton>
          <span class="muted">{{ rows.length ? state.offset + 1 : 0 }}–{{ Math.min(state.offset + 50, rows.length) }} / {{ rows.length }}</span>
          <ZButton :disabled="state.offset + 50 >= rows.length" @click="state.offset += 50">下一页</ZButton>
        </div>
      </section>
    </template>
    <ZDialog v-model="confirmation.open" title="放弃未保存修改" type="warning">
      <p>当前 Key 的未保存修改将被放弃</p>
      <template #footer>
        <ZButton @click="confirmation.open = false">继续编辑</ZButton>
        <ZButton type="primary" @click="discard">放弃并读取</ZButton>
      </template>
    </ZDialog>
    <ZDialog v-model="blockAll.open" title="禁止所有模型" type="warning">
      <p>此 Key 将无法请求任何模型，模型列表也将为空</p>
      <template #footer>
        <ZButton @click="blockAll.open = false">取消</ZButton>
        <ZButton type="danger" @click="save">确认保存</ZButton>
      </template>
    </ZDialog>
  </section>
</template>
