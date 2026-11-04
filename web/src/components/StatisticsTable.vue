<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { Dimension, Parent, StatPage, StatQuery, StatRow, StatSort, Totals } from '../api/statistics'
import { ZButton, ZInput, ZSelect, ZTable } from '@codex-proxy/ui'
import { onBeforeUnmount, reactive, watch } from 'vue'
import { message } from '../api'
import { costDetails, exportStatistics, formatCosts, formatCount, formatMetric, quality, queryStatistics } from '../api/statistics'
import TablePagination from './TablePagination.vue'

const props = withDefaults(defineProps<{
  taskId: string
  dimension: Dimension
  parent?: Parent | null
  inspect?: boolean
}>(), { parent: null, inspect: true })
const emit = defineEmits<{ inspect: [row: StatRow], summary: [value: Totals] }>()
const state = reactive({ page: null as StatPage | null, draft: '', search: '', sort: 'cost' as StatSort,
  ascending: false, busy: false, exporting: false, error: '' })
let revision = 0
const sortOptions = [{ label: '金额 USD', value: 'cost' }, { label: '总 Token', value: 'tokens' }, { label: '请求次数', value: 'requests' }]
const columns: TableColumn<StatRow>[] = [
  { key: 'name', label: '名称', kind: 'custom', size: 'xl' },
  { key: 'requests', label: '请求数', kind: 'custom', size: 'sm' },
  { key: 'tokens', label: '总 Token', kind: 'custom', size: 'md' },
  { key: 'input', label: '输入', kind: 'custom', size: 'md' },
  { key: 'output', label: '输出', kind: 'custom', size: 'md' },
  { key: 'cacheRead', label: '缓存读', kind: 'custom', size: 'md' },
  { key: 'cacheWrite', label: '缓存写', kind: 'custom', size: 'md' },
  { key: 'cacheRatio', label: '缓存读占比', kind: 'custom', size: 'sm' },
  { key: 'cost', label: '已知费用', kind: 'custom', size: 'lg' },
  ...(props.inspect ? [{ key: 'actions', label: '明细', kind: 'custom' as const, size: 'sm' as const }] : []),
]
function query(offset = 0): StatQuery {
  return { taskId: props.taskId, dimension: props.dimension, parent: props.parent, offset,
    search: state.search, sort: state.sort, ascending: state.ascending }
}
async function load(offset = 0) {
  const current = ++revision
  state.busy = true
  state.error = ''
  try {
    const page = await queryStatistics(query(offset))
    if (current !== revision) return
    state.page = page
    emit('summary', page.summary)
  }
  catch (error) {
    if (current === revision) { state.error = message(error); state.page = null }
  }
  finally { if (current === revision) state.busy = false }
}
function search() { state.search = state.draft.trim(); void load() }
async function download() {
  const current = revision
  state.exporting = true
  state.error = ''
  try {
    const { csv, filename } = await exportStatistics(query())
    if (current !== revision) return
    const url = URL.createObjectURL(new Blob([csv], { type: 'text/csv;charset=utf-8' }))
    const link = document.createElement('a')
    link.href = url
    link.download = filename
    document.body.append(link)
    link.click()
    link.remove()
    URL.revokeObjectURL(url)
  }
  catch (error) { state.error = message(error) }
  finally { state.exporting = false }
}
function outcomes(value: Totals) {
  return `成功 ${formatCount(value.succeeded)} · 失败 ${formatCount(value.failed)} · 取消 ${formatCount(value.cancelled)} · 未完成 ${formatCount(value.incomplete)} · 运行中 ${formatCount(value.running)}${value.unknownOutcomes !== '0' ? ` · 终态未归属 ${formatCount(value.unknownOutcomes)}` : ''}`
}
watch(() => [props.taskId, props.dimension, props.parent?.dimension, props.parent?.id], () => {
  state.draft = ''; state.search = ''; state.page = null; void load()
}, { immediate: true })
onBeforeUnmount(() => { revision += 1 })
</script>

<template>
  <div class="panel">
    <div class="toolbar">
      <ZInput v-model="state.draft" aria-label="搜索名称或 ID" placeholder="搜索名称或 ID" class="stat-search" @keydown.enter="search" />
      <ZButton :disabled="state.busy" @click="search">筛选</ZButton>
      <ZSelect v-model="state.sort" :options="sortOptions" aria-label="排序指标" class="stat-sort" :disabled="state.busy" @change="load()" />
      <ZButton :disabled="state.busy" @click="state.ascending = !state.ascending; load()">{{ state.ascending ? '升序' : '降序' }}</ZButton>
      <ZButton class="stat-export" :loading="state.exporting" :disabled="state.busy || !state.page?.total" @click="download">导出 CSV</ZButton>
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <p v-if="state.busy" class="muted" role="status">正在汇总分组</p>
    <ZTable :columns="columns" :data="state.page?.items ?? []" row-key="id" :empty-text="state.busy ? '正在读取' : '此范围没有匹配记录'">
      <template #name="{ row }">
        <div class="stat-name">
          <span>{{ row.name }}</span>
          <span v-if="row.provider" class="muted">{{ row.provider }}</span>
          <span v-if="dimension === 'key'" class="muted stat-id">{{ row.id || '无 Key 归属' }}</span>
          <span v-if="quality(row.totals)" class="muted">{{ quality(row.totals) }}</span>
          <span v-if="!inspect && dimension === 'model' && row.aliases.length" class="muted">请求模型：{{ row.aliases.join(' / ') }}</span>
        </div>
      </template>
      <template #requests="{ row }"><details class="stat-outcomes"><summary>{{ formatCount(row.totals.requests) }}</summary><p>{{ outcomes(row.totals) }}</p></details></template>
      <template #tokens="{ row }">{{ formatMetric(row.totals.tokens) }}</template>
      <template #input="{ row }">{{ formatMetric(row.totals.input) }}</template>
      <template #output="{ row }">{{ formatMetric(row.totals.output) }}</template>
      <template #cacheRead="{ row }">{{ formatMetric(row.totals.cacheRead) }}</template>
      <template #cacheWrite="{ row }">{{ formatMetric(row.totals.cacheWrite) }}</template>
      <template #cacheRatio="{ row }">{{ row.totals.cacheRatio === null ? '未知' : `${(row.totals.cacheRatio * 100).toFixed(1)}%` }}</template>
      <template #cost="{ row }"><span :title="costDetails(row.totals)">{{ formatCosts(row.totals.costs) }}</span></template>
      <template #actions="{ row }"><ZButton variant="text" type="primary" @click="emit('inspect', row)">{{ dimension === 'key' ? '查看模型' : '查看 Key' }}</ZButton></template>
    </ZTable>
    <TablePagination v-if="state.page" :offset="state.page.offset" :total="state.page.total" :page-size="state.page.pageSize" :busy="state.busy" @change="load" />
  </div>
</template>

<style scoped>
.stat-search { width: min(100%, 20rem); }
.stat-sort { width: 10rem; }
.stat-export { margin-left: auto; }
.stat-name { display: grid; gap: 4px; overflow-wrap: anywhere; }
.stat-id { font-family: monospace; }
.stat-outcomes summary { cursor: pointer; font-variant-numeric: tabular-nums; }
.stat-outcomes p { white-space: normal; min-width: 10rem; }
</style>
