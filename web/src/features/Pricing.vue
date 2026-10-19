<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { Catalog, CommunityPrice, HistoryFilter, OperationResult, TokenPrice } from '../api'
import { ZButton, ZDialog, ZInput, ZRadio, ZSelect, ZTable } from '@codex-proxy/ui'
import { computed, onMounted, reactive } from 'vue'
import {
  array,
  getCommunity,
  getPricing,
  json,
  message,
  operationResults,
  prices,
  request,
  syncHistoryPage,
  text,
} from '../api'
import OperationResults from '../components/OperationResults.vue'
import { localDateTime } from '../dates'

interface Row {
  id: string
  provider: string
  model: string
  source: string
  builtin: boolean
  input: string | null
  output: string | null
  cacheRead: string | null
  cacheWrite: string | null
}
interface Draft extends TokenPrice {
  id: string
  provider: string
  model: string
}
const state = reactive({
  catalog: null as Catalog | null,
  provider: 'openai',
  search: '',
  selected: {} as Record<string, boolean>,
  busy: false,
  error: '',
  notice: '',
  results: [] as OperationResult[],
  action: '',
})
const editor = reactive({
  open: false,
  models: '',
  provider: 'openai',
  input: '',
  output: '',
  cacheRead: '',
  cacheWrite: '',
  multiplierBps: '10000',
  error: '',
})
const community = reactive({
  open: false,
  provider: 'google',
  search: '',
  modelFilter: '',
  appliedSearch: '',
  appliedModelFilter: '',
  target: 'openai',
  providers: [] as { id: string; name: string }[],
  rows: [] as CommunityPrice[],
  offset: 0,
  total: 0,
  skipped: 0,
  selected: {} as Record<string, boolean>,
  busy: false,
  error: '',
})
const drafts = reactive({ rows: [] as Draft[], open: false, error: '' })
const history = reactive({
  open: false,
  mode: 'missing',
  notice: '',
  error: '',
  busy: false,
  pause: false,
  start: localDateTime(new Date(Date.now() - 30 * 86400000)),
  end: localDateTime(),
  provider: 'openai',
  filter: null as HistoryFilter | null,
  cursor: null as string | null,
  done: false,
  scanned: 0,
  saved: 0,
  skipped: 0,
  results: [] as { id: string; status: string; message: string }[],
})
const sync = reactive({
  open: false,
  preview: null as Record<string, unknown> | null,
  rows: [] as Row[],
  selected: {} as Record<string, boolean>,
  skipped: [] as string[],
  offset: 0,
})
const columns: TableColumn<Row>[] = [
  { key: 'select', label: '选择', kind: 'selection', width: 80 },
  { key: 'model', label: '上游模型 ID', kind: 'mono', size: 'xl' },
  { key: 'source', label: '价目来源', size: 'sm' },
  { key: 'input', label: '输入', kind: 'numeric', emptyText: '未提供' },
  { key: 'output', label: '输出', kind: 'numeric', emptyText: '未提供' },
  { key: 'cacheRead', label: '缓存读', kind: 'numeric', emptyText: '未提供' },
  { key: 'cacheWrite', label: '缓存写', kind: 'numeric', emptyText: '未提供' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'sm' },
]
const rows = computed((): Row[] => {
  if (!state.catalog) return []
  const catalog = state.catalog
  const p = state.provider
  const models = new Set([
    ...Object.keys(catalog.defaults[p] ?? {}),
    ...Object.keys(catalog.synced[p] ?? {}),
    ...Object.keys(catalog.overrides[p] ?? {}),
  ])
  return [...models]
    .sort()
    .filter((m) => m.toLowerCase().includes(state.search.toLowerCase()))
    .map((model) => {
      const band =
        catalog.overrides[p]?.[model]?.bands.standard ??
        catalog.synced[p]?.[model]?.bands.standard ??
        catalog.defaults[p]?.[model]?.bands.standard
      return {
        id: `${p}:${model}`,
        provider: p,
        model,
        source: catalog.overrides[p]?.[model]
          ? '人工覆盖'
          : catalog.synced[p]?.[model]
            ? '宿主同步'
            : '内置',
        builtin: !!catalog.defaults[p]?.[model],
        input: band?.input ?? null,
        output: band?.output ?? null,
        cacheRead: band?.cacheRead ?? '0',
        cacheWrite: band?.cacheWrite ?? '0',
      }
    })
})
const selected = computed(() => rows.value.filter((row) => state.selected[row.id]))
const syncPage = computed(() => sync.rows.slice(sync.offset, sync.offset + 100))
const mainSelection = computed(() =>
  selectionState(
    rows.value.map((row) => row.id),
    state.selected,
  ),
)
const communitySelection = computed(() =>
  selectionState(
    community.rows.map((row) => row.model),
    community.selected,
  ),
)
const syncSelection = computed(() =>
  selectionState(
    syncPage.value.map((row) => row.id),
    sync.selected,
  ),
)
function selectionState(ids: string[], selected: Record<string, boolean>) {
  const count = ids.filter((id) => selected[id]).length
  return {
    ids,
    checked: ids.length > 0 && count === ids.length,
    partial: count > 0 && count < ids.length,
  }
}
function selectAll(ids: string[], selected: Record<string, boolean>, event: Event) {
  const checked = (event.target as HTMLInputElement).checked
  for (const id of ids) selected[id] = checked
}
async function load() {
  state.busy = true
  state.error = ''
  try {
    state.catalog = await getPricing()
    state.selected = {}
  } catch (error) {
    state.catalog = null
    state.error = message(error)
  } finally {
    state.busy = false
  }
}
function edit(row?: Row) {
  editor.provider = row?.provider ?? state.provider
  editor.models = row ? row.model : selected.value.map((r) => r.model).join('\n')
  const sample = row ?? selected.value[0]
  editor.input = sample?.input ?? ''
  editor.output = sample?.output ?? ''
  editor.cacheRead = sample?.cacheRead ?? '0'
  editor.cacheWrite = sample?.cacheWrite ?? '0'
  editor.multiplierBps = sample
    ? String(state.catalog?.overrides[sample.provider]?.[sample.model]?.multiplierBps ?? 10000)
    : '10000'
  editor.error = ''
  editor.open = true
}
function priceInputValid(values: string[]): boolean {
  return values.every((value) => /^\d+(?:\.\d+)?$/.test(value))
}
function cachePrice(value: string): string {
  return value.trim() || '0'
}
async function write(commands: unknown[]) {
  state.busy = true
  state.error = ''
  state.notice = ''
  state.results = []
  try {
    state.results = await request('POST', 'api/pricing/update', commands, operationResults)
    state.catalog = await getPricing()
    state.selected = {}
  } catch (error) {
    state.catalog = null
    state.error = `${message(error)}；请重新读取价目，不要直接重复提交`
  } finally {
    state.busy = false
  }
}
function command(provider: string, model: string, band: TokenPrice, multiplierBps = 10000) {
  const old = state.catalog?.overrides[provider]?.[model]
  return {
    provider,
    models: [model],
    action: 'replace',
    pricing: {
      multiplierBps,
      bands: {
        ...old?.bands,
        standard: {
          input: band.input,
          output: band.output,
          cacheRead: cachePrice(band.cacheRead),
          cacheWrite: cachePrice(band.cacheWrite),
        },
      },
    },
    multiplierBps: null,
  }
}
async function saveEditor() {
  editor.cacheRead = cachePrice(editor.cacheRead)
  editor.cacheWrite = cachePrice(editor.cacheWrite)
  const models = editor.models
    .split(/\r?\n/)
    .map((m) => m.trim())
    .filter(Boolean)
  const multiplier = Number(editor.multiplierBps)
  if (
    !models.length ||
    models.length > 500 ||
    new Set(models).size !== models.length ||
    !priceInputValid([editor.input, editor.output, editor.cacheRead, editor.cacheWrite]) ||
    !/^\d+$/.test(editor.multiplierBps) ||
    !Number.isSafeInteger(multiplier) ||
    multiplier > 1000000
  ) {
    editor.error = '请核对模型 ID、四项价格和倍率，最多 500 个不重复模型'
    return
  }
  editor.open = false
  const band: TokenPrice = {
    input: editor.input,
    output: editor.output,
    cacheRead: editor.cacheRead,
    cacheWrite: editor.cacheWrite,
  }
  await write(models.map((model) => command(editor.provider, model, band, multiplier)))
}
async function applyAction() {
  const action = state.action
  state.action = ''
  await write([
    {
      provider: state.provider,
      models: selected.value.map((r) => r.model),
      action,
      pricing: null,
      multiplierBps: null,
    },
  ])
}
async function loadCommunity(offset = 0, applyFilters = true) {
  const search = applyFilters ? community.search : community.appliedSearch
  const modelFilter = applyFilters ? community.modelFilter : community.appliedModelFilter
  community.busy = true
  community.error = ''
  try {
    const result = await getCommunity(community.provider, offset, search, modelFilter)
    community.providers = result.providers
    community.rows = result.rows
    community.total = result.total
    community.skipped = result.skipped
    community.offset = offset
    community.selected = {}
    community.appliedSearch = search
    community.appliedModelFilter = modelFilter
  } catch (error) {
    community.rows = []
    community.total = 0
    community.skipped = 0
    community.offset = 0
    community.selected = {}
    community.error = message(error)
  } finally {
    community.busy = false
  }
}
async function clearCommunityFilters() {
  community.search = ''
  community.modelFilter = ''
  await loadCommunity()
}
async function openCommunity() {
  community.open = true
  await loadCommunity()
}
function openHistory() {
  if (!history.filter || history.done) {
    history.mode = 'missing'
    history.provider = state.provider
    history.end = localDateTime()
  }
  history.open = true
}
async function confirmHistory() {
  if (!history.filter || history.done) {
    const startMs = Date.parse(history.start)
    const endMs = Date.parse(history.end)
    if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || startMs >= endMs) {
      history.error = '请选择有效的起止时间'
      return
    }
    history.filter = { startMs, endMs, provider: history.provider, mode: history.mode }
    history.cursor = null
    history.scanned = history.saved = history.skipped = 0
    history.results = []
    history.done = false
  }
  history.open = false
  history.busy = true
  state.busy = true
  history.pause = false
  history.error = ''
  history.notice = ''
  try {
    do {
      const page = await syncHistoryPage(history.filter, history.cursor)
      history.scanned += page.scanned
      history.saved += page.results.filter((row) => row.status === 'saved').length
      history.skipped += page.results.filter((row) => row.status === 'skipped').length
      history.results = page.results
      history.cursor = page.nextCursor
      history.done = page.nextCursor === null
    } while (!history.done && !history.pause)
    history.notice = history.done ? '历史 Token 费用同步完成' : '已暂停，可继续本次范围的同步'
  } catch (error) {
    history.error = `${message(error)}；后续页已停止，继续时重新核对当前页，已确认数量不含本次未知结果`
  } finally {
    history.busy = false
    state.busy = false
  }
}
function addCommunity() {
  const selected = community.rows.filter((row) => community.selected[row.model])
  if (!selected.length) {
    community.error = '请选择社区价目'
    return
  }
  if (
    drafts.rows.length + selected.length > 500 ||
    selected.some((row) =>
      drafts.rows.some((d) => d.provider === community.target && d.model === row.model),
    )
  ) {
    community.error = '待写列表已有同名模型或超过 500 项，请先处理列表'
    return
  }
  drafts.rows.push(
    ...selected.map((row) => ({
      id: `${community.target}:${row.sourceProvider}:${row.model}`,
      provider: community.target,
      model: row.model,
      input: row.input,
      output: row.output,
      cacheRead: row.cacheRead,
      cacheWrite: row.cacheWrite,
    })),
  )
  community.open = false
  drafts.open = true
  drafts.error = ''
}
async function saveDrafts() {
  for (const row of drafts.rows) {
    row.cacheRead = cachePrice(row.cacheRead)
    row.cacheWrite = cachePrice(row.cacheWrite)
  }
  const ids = drafts.rows.map((row) => `${row.provider}:${row.model.trim()}`)
  if (
    !drafts.rows.length ||
    new Set(ids).size !== ids.length ||
    drafts.rows.some(
      (row) =>
        !row.model.trim() ||
        !priceInputValid([row.input, row.output, row.cacheRead, row.cacheWrite]),
    )
  ) {
    drafts.error = '请填写不重复的实际上游模型 ID 和有效价格，输入与输出价格必填'
    return
  }
  drafts.open = false
  await write(drafts.rows.map((row) => command(row.provider, row.model.trim(), row)))
  if (
    state.results.length === drafts.rows.length &&
    state.results.every((row) => row.status === 'saved')
  ) {
    drafts.rows = []
  }
}
async function previewSync() {
  state.busy = true
  state.error = ''
  try {
    const preview = await json('POST', 'api/pricing/preview-sync', {})
    const catalog = prices(preview.prices)
    sync.preview = preview
    sync.skipped = array(preview.skipped, text)
    sync.selected = {}
    sync.offset = 0
    sync.rows = Object.entries(catalog).flatMap(([provider, models]) =>
      Object.entries(models).map(([model, price]): Row => ({
        id: `${provider}:${model}`,
        provider,
        model,
        source: '同步预览',
        builtin: false,
        input: price.bands.standard?.input ?? null,
        output: price.bands.standard?.output ?? null,
        cacheRead: price.bands.standard?.cacheRead ?? '0',
        cacheWrite: price.bands.standard?.cacheWrite ?? '0',
      })),
    )
    sync.open = true
  } catch (error) {
    state.error = message(error)
  } finally {
    state.busy = false
  }
}
async function saveSync() {
  const models: Record<string, string[]> = {}
  for (const row of sync.rows.filter((row) => sync.selected[row.id]))
    (models[row.provider] ??= []).push(row.model)
  if (!Object.keys(models).length) {
    state.error = '请选择要同步的模型'
    return
  }
  sync.open = false
  state.busy = true
  state.error = ''
  try {
    await json('POST', 'api/pricing/sync', { preview: sync.preview, models })
    state.catalog = await getPricing()
    state.notice = '选中模型的来源价目已同步，人工覆盖保持原样'
  } catch (error) {
    state.catalog = null
    state.error = `${message(error)}；请重新预览和读取价目核对`
  } finally {
    state.busy = false
  }
}
const communityColumns: TableColumn<CommunityPrice>[] = [
  { key: 'select', label: '选择', kind: 'selection', width: 80 },
  { key: 'model', label: '社区模型 ID', size: 'xl', kind: 'mono' },
  { key: 'input', label: '输入', kind: 'numeric' },
  { key: 'output', label: '输出', kind: 'numeric' },
  { key: 'cacheRead', label: '缓存读', kind: 'numeric', emptyText: '未提供' },
  { key: 'cacheWrite', label: '缓存写', kind: 'numeric', emptyText: '未提供' },
]
const draftColumns: TableColumn<Draft>[] = [
  { key: 'model', label: '实际上游模型 ID', size: 'xl', kind: 'custom' },
  { key: 'input', label: '输入', size: 'md', kind: 'custom' },
  { key: 'output', label: '输出', size: 'md', kind: 'custom' },
  { key: 'cacheRead', label: '缓存读', size: 'md', kind: 'custom' },
  { key: 'cacheWrite', label: '缓存写', size: 'md', kind: 'custom' },
  { key: 'actions', label: '操作', size: 'sm', kind: 'actions' },
]
onMounted(load)
</script>

<template>
  <section class="panel">
    <p class="muted">CPR 全局价目 · USD / 每百万 Token · 改价影响后续估算，不重算历史费用</p>
    <div class="toolbar">
      <ZSelect
        :model-value="state.provider"
        aria-label="计价 Provider"
        :options="[
          { value: 'openai', label: 'OpenAI 兼容' },
          { value: 'xai', label: '原生 xAI' },
        ]"
        style="width: 180px"
        @update:model-value="state.provider = $event ?? ''"
      /><ZInput
        v-model="state.search"
        aria-label="筛选模型"
        placeholder="筛选模型"
        style="max-width: 280px"
      /><ZButton :loading="state.busy" @click="load"> 重新读取 </ZButton
      ><ZButton :disabled="state.busy || !state.catalog || selected.length > 500" @click="edit()">
        新增或批量编辑 </ZButton
      ><ZButton :disabled="state.busy || !state.catalog" @click="previewSync">
        宿主同步预览 </ZButton
      ><ZButton :disabled="state.busy" @click="openCommunity"> models.dev 社区价目 </ZButton
      ><ZButton :disabled="state.busy || history.busy" @click="openHistory"> 价格同步 </ZButton>
    </div>
    <div class="toolbar">
      <ZButton
        :disabled="!selected.length || selected.length > 500 || state.busy"
        @click="state.action = 'reset'"
        >重置选中覆盖</ZButton
      ><ZButton
        type="danger"
        variant="text"
        :disabled="
          !selected.length || selected.length > 500 || selected.some((r) => r.builtin) || state.busy
        "
        @click="state.action = 'delete'"
        >删除选中价目</ZButton
      ><ZButton
        v-if="drafts.rows.length"
        :disabled="state.busy || !state.catalog"
        @click="drafts.open = true"
        >待写列表 {{ drafts.rows.length }}</ZButton
      ><span class="muted">已选 {{ selected.length }} 个 · 内置模型不能删除</span>
    </div>
    <p v-if="state.error" class="error" role="alert">
      {{ state.error }}
    </p>
    <p v-if="state.notice" class="success" role="status">
      {{ state.notice }}
    </p>
    <p v-if="history.notice" class="notice" role="status">
      {{ history.notice }}
    </p>
    <p v-if="history.error" class="error" role="alert">
      {{ history.error }}
    </p>
    <div v-if="history.filter" class="section">
      <p role="status">
        已确认扫描 {{ history.scanned }} 条 · 写回 {{ history.saved }} 条 · 跳过
        {{ history.skipped }} 条
      </p>
      <div class="toolbar">
        <ZButton v-if="history.busy" :disabled="history.pause" @click="history.pause = true">
          暂停后续页
        </ZButton>
        <ZButton v-else-if="!history.done" @click="openHistory"> 继续核对同步 </ZButton>
      </div>
      <details v-if="history.results.length">
        <summary>最近一页处理结果</summary>
        <ul>
          <li v-for="row in history.results" :key="row.id">
            <span class="mono">{{ row.id }}</span> · {{ row.message }}
          </li>
        </ul>
      </details>
    </div>
    <p v-if="selected.length > 500" class="notice">
      单次最多写入 500 个模型，请缩小筛选范围后重新全选
    </p>
    <ZTable
      :columns="columns"
      :data="rows"
      row-key="id"
      :loading="state.busy"
      size="small"
      empty-text="暂无价目，可按实际上游模型 ID 新增"
      ><template #header-select>
        <label class="table-select-all"
          ><input
            type="checkbox"
            class="native-check"
            :checked="mainSelection.checked"
            :indeterminate="mainSelection.partial"
            :disabled="state.busy || !rows.length"
            aria-label="全选当前筛选结果"
            @change="selectAll(mainSelection.ids, state.selected, $event)"
          /><span>全选</span></label
        > </template
      ><template #select="{ row }"
        ><input
          v-model="state.selected[row.id]"
          type="checkbox"
          class="native-check"
          :aria-label="`选择 ${row.model}`"
          :disabled="state.busy" /></template
      ><template #actions="{ row }"
        ><ZButton variant="text" size="small" :disabled="state.busy" @click="edit(row)"
          >编辑</ZButton
        ></template
      ></ZTable
    >
    <OperationResults :results="state.results" />
    <ZDialog v-model="editor.open" title="新增或批量编辑标准价"
      ><div class="panel">
        <label class="field"
          ><span>Provider</span
          ><ZSelect
            :model-value="editor.provider"
            :options="[
              { value: 'openai', label: 'OpenAI 兼容' },
              { value: 'xai', label: '原生 xAI' },
            ]"
            @update:model-value="editor.provider = $event ?? ''" /></label
        ><label class="field"
          ><span>实际上游模型 ID，每行一个</span
          ><textarea v-model="editor.models" rows="4" class="mono" aria-label="实际上游模型 ID" />
        </label>
        <p class="muted">这组模型使用相同标准价，其他人工档位保留 · 缓存价格留空按 0 写入</p>
        <div class="form-grid">
          <label
            v-for="field in ['input', 'output', 'cacheRead', 'cacheWrite'] as const"
            :key="field"
            class="field"
            ><span>{{
              {
                input: '输入',
                output: '输出',
                cacheRead: '缓存读',
                cacheWrite: '缓存写',
              }[field]
            }}</span
            ><ZInput
              v-model="editor[field]"
              inputmode="decimal"
              :placeholder="
                field === 'cacheRead' || field === 'cacheWrite' ? '0' : '未提供，需填写'
              " /></label
          ><label class="field"
            ><span>倍率基点，10000 为 1 倍</span
            ><ZInput v-model="editor.multiplierBps" inputmode="numeric"
          /></label>
        </div>
        <p v-if="editor.error" class="error" role="alert">{{ editor.error }}</p>
      </div>
      <template #footer
        ><ZButton @click="editor.open = false">取消</ZButton
        ><ZButton type="primary" @click="saveEditor">确认写入</ZButton></template
      ></ZDialog
    >
    <ZDialog
      :model-value="!!state.action"
      title="确认批量操作"
      type="warning"
      @update:model-value="!$event && (state.action = '')"
      ><div class="panel">
        <p>
          {{ state.action === 'delete' ? '删除' : '重置' }}选中的 {{ selected.length }} 个模型价目
        </p>
        <p class="muted">不会修改历史费用</p>
      </div>
      <template #footer
        ><ZButton @click="state.action = ''">取消</ZButton
        ><ZButton type="primary" @click="applyAction">确认操作</ZButton></template
      ></ZDialog
    >
    <ZDialog v-model="history.open" title="同步历史费用" type="warning">
      <div class="panel">
        <p>按当前 CPR 全局价目试算并写回选定范围的原生请求费用</p>
        <div class="form-grid">
          <label class="field"
            ><span>开始时间（本机时区）</span
            ><input
              v-model="history.start"
              type="datetime-local"
              :disabled="!!history.filter && !history.done"
          /></label>
          <label class="field"
            ><span>结束时间（不含）</span
            ><input
              v-model="history.end"
              type="datetime-local"
              :disabled="!!history.filter && !history.done"
          /></label>
          <label class="field"
            ><span>Provider</span
            ><ZSelect
              :model-value="history.provider"
              :options="[
                { value: 'openai', label: 'OpenAI 兼容' },
                { value: 'xai', label: '原生 xAI' },
              ]"
              :disabled="!!history.filter && !history.done"
              @update:model-value="history.provider = $event ?? ''"
          /></label>
        </div>
        <div class="field" role="radiogroup" aria-label="历史费用同步方式">
          <ZRadio
            :model-value="history.mode"
            name="history-pricing-mode"
            value="missing"
            :disabled="!!history.filter && !history.done"
            @update:model-value="history.mode = String($event)"
          >
            只补全缺失费用
          </ZRadio>
          <p class="muted">保留已有费用，0 费用不视为缺失</p>
          <ZRadio
            :model-value="history.mode"
            name="history-pricing-mode"
            value="all"
            :disabled="!!history.filter && !history.done"
            @update:model-value="history.mode = String($event)"
          >
            覆盖范围内全部历史费用
          </ZRadio>
          <p class="muted">按当前价目重新计算并替换已有费用</p>
        </div>
        <p class="notice">
          仅计算 Token 费用，不包含工具等附加费用 · 覆盖会替换原费用并清除原计价快照，不改变 Key
          预算
        </p>
        <p class="muted">
          缺少 Token、图像用量或无法计价的记录跳过 · CCS 导入的聚合贡献不参与费用重算
        </p>
        <p v-if="history.error" class="error" role="alert">
          {{ history.error }}
        </p>
      </div>
      <template #footer>
        <ZButton @click="history.open = false"> 取消 </ZButton>
        <ZButton type="primary" @click="confirmHistory">
          {{ history.filter && !history.done ? '继续核对同步' : '确认同步' }}
        </ZButton>
      </template>
    </ZDialog>
    <ZDialog v-model="community.open" title="models.dev 社区价目" width="min(1100px, 94vw)"
      ><div class="panel">
        <p class="notice">
          社区价目仅供参考，不代表中转账单 · 模型 ID 不一定与中转相同，加入列表后核对再写入
        </p>
        <div class="toolbar">
          <ZSelect
            :model-value="community.provider"
            :options="community.providers.map((p) => ({ value: p.id, label: p.name }))"
            aria-label="社区供应商"
            filterable
            :disabled="community.busy"
            style="width: 240px"
            @update:model-value="community.provider = $event ?? ''"
            @change="loadCommunity()"
          /><ZSelect
            :model-value="community.target"
            :options="[
              { value: 'openai', label: '写入 OpenAI 兼容价目' },
              { value: 'xai', label: '写入原生 xAI 价目' },
            ]"
            aria-label="目标 Provider"
            style="width: 240px"
            @update:model-value="community.target = $event ?? ''"
          /><ZButton :loading="community.busy" @click="loadCommunity(community.offset, false)">
            重新获取
          </ZButton>
        </div>
        <div class="toolbar">
          <ZInput
            v-model="community.search"
            aria-label="社区价目表格筛选"
            placeholder="筛选名称、模型 ID 或价格"
            maxlength="256"
            :disabled="community.busy"
            style="max-width: 280px"
            @keydown.enter="loadCommunity()"
          />
          <ZButton :disabled="community.busy" @click="loadCommunity()">筛选</ZButton>
          <ZButton
            variant="text"
            :disabled="
              community.busy ||
              (!community.search &&
                !community.modelFilter &&
                !community.appliedSearch &&
                !community.appliedModelFilter)
            "
            @click="clearCommunityFilters"
          >
            清空筛选
          </ZButton>
        </div>
        <p
          v-if="
            community.search !== community.appliedSearch ||
            community.modelFilter !== community.appliedModelFilter
          "
          class="muted"
        >
          筛选条件已修改，点击「筛选」或按 Enter 应用
        </p>
        <p v-if="community.error" class="error" role="alert">
          {{ community.error }}
        </p>
        <ZTable
          :columns="communityColumns"
          :data="community.rows"
          row-key="model"
          :loading="community.busy"
          size="small"
          empty-text="当前筛选没有可用价目"
          ><template #header-model>
            <label class="field">
              <span>社区模型 ID</span>
              <ZInput
                v-model="community.modelFilter"
                aria-label="筛选社区模型 ID"
                placeholder="模型 ID 包含"
                maxlength="256"
                size="small"
                :disabled="community.busy"
                @keydown.enter.stop="loadCommunity()"
              />
            </label> </template
          ><template #header-select>
            <label class="table-select-all"
              ><input
                type="checkbox"
                class="native-check"
                :checked="communitySelection.checked"
                :indeterminate="communitySelection.partial"
                :disabled="community.busy || !community.rows.length"
                aria-label="全选本页社区价目"
                @change="selectAll(communitySelection.ids, community.selected, $event)"
              /><span>全选</span></label
            > </template
          ><template #select="{ row }"
            ><input
              v-model="community.selected[row.model]"
              type="checkbox"
              class="native-check"
              :disabled="community.busy"
              :aria-label="`选择 ${row.model}`" /></template
        ></ZTable>
        <div class="toolbar">
          <ZButton
            :disabled="community.busy || community.offset === 0"
            @click="loadCommunity(community.offset - 100, false)"
            >上一页</ZButton
          ><span
            >{{ community.total ? community.offset + 1 : 0 }}–{{
              Math.min(community.offset + 100, community.total)
            }}
            / {{ community.total }}</span
          ><ZButton
            :disabled="community.busy || community.offset + 100 >= community.total"
            @click="loadCommunity(community.offset + 100, false)"
            >下一页</ZButton
          ><span class="muted">筛选结果中跳过不支持的价目 {{ community.skipped }} 项</span>
        </div>
      </div>
      <template #footer
        ><ZButton @click="community.open = false">关闭</ZButton
        ><ZButton type="primary" :disabled="community.busy || !state.catalog" @click="addCommunity"
          >加入待写列表</ZButton
        ></template
      ></ZDialog
    >
    <ZDialog v-model="drafts.open" title="核对社区价目" width="min(1200px, 94vw)"
      ><div class="panel">
        <p class="muted">目标模型 ID 必须与实际请求一致 · 缓存价格缺失或留空时按 0 写入</p>
        <ZTable :columns="draftColumns" :data="drafts.rows" row-key="id" size="small"
          ><template #model="{ row }"
            ><ZInput v-model="row.model" :aria-label="`目标模型 ${row.id}`" /></template
          ><template
            v-for="field in ['input', 'output', 'cacheRead', 'cacheWrite'] as const"
            :key="field"
            #[field]="{ row }"
            ><ZInput
              v-model="row[field]"
              inputmode="decimal"
              :aria-label="`${row.model} ${field}`"
              :placeholder="
                field === 'cacheRead' || field === 'cacheWrite' ? '0' : '未提供'
              " /></template
          ><template #actions="{ index }"
            ><ZButton
              type="danger"
              variant="text"
              size="small"
              @click="drafts.rows.splice(index, 1)"
              >移除</ZButton
            ></template
          ></ZTable
        >
        <p v-if="drafts.error" class="error" role="alert">{{ drafts.error }}</p>
      </div>
      <template #footer
        ><ZButton @click="drafts.open = false">保留待写列表</ZButton
        ><ZButton type="primary" :disabled="state.busy || !state.catalog" @click="saveDrafts"
          >确认写入 {{ drafts.rows.length }} 项</ZButton
        ></template
      ></ZDialog
    >
    <ZDialog v-model="sync.open" title="宿主同步预览" width="min(1100px, 94vw)"
      ><div class="panel">
        <p class="muted">同步只更新来源层，人工覆盖保持原样</p>
        <ZTable
          :columns="columns.filter((c) => c.key !== 'actions')"
          :data="syncPage"
          row-key="id"
          size="small"
          ><template #header-select>
            <label class="table-select-all"
              ><input
                type="checkbox"
                class="native-check"
                :checked="syncSelection.checked"
                :indeterminate="syncSelection.partial"
                :disabled="!syncPage.length"
                aria-label="全选本页同步价目"
                @change="selectAll(syncSelection.ids, sync.selected, $event)"
              /><span>全选</span></label
            > </template
          ><template #select="{ row }"
            ><input
              v-model="sync.selected[row.id]"
              type="checkbox"
              class="native-check"
              :aria-label="`同步 ${row.provider} ${row.model}`" /></template
          ><template #model="{ row }"
            ><span class="mono">{{ row.provider }}/{{ row.model }}</span></template
          ></ZTable
        >
        <div class="toolbar">
          <ZButton :disabled="sync.offset === 0" @click="sync.offset -= 100">上一页</ZButton
          ><span
            >{{ sync.offset + 1 }}–{{ Math.min(sync.offset + 100, sync.rows.length) }} /
            {{ sync.rows.length }}</span
          ><ZButton :disabled="sync.offset + 100 >= sync.rows.length" @click="sync.offset += 100"
            >下一页</ZButton
          >
        </div>
        <details v-if="sync.skipped.length">
          <summary>来源跳过 {{ sync.skipped.length }} 项</summary>
          <ul>
            <li v-for="model in sync.skipped" :key="model">{{ model }}</li>
          </ul>
        </details>
      </div>
      <template #footer
        ><ZButton @click="sync.open = false">取消</ZButton
        ><ZButton type="primary" @click="saveSync">确认同步选中模型</ZButton></template
      ></ZDialog
    >
  </section>
</template>
