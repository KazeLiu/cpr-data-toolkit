<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { Dimension, Parent, StatPage, StatQuery, StatRow, StatSort, Totals } from '../api/statistics'
import { ZButton, ZInput, ZSelect, ZTable } from '@codex-proxy/ui'
import { onBeforeUnmount, reactive, watch } from 'vue'
import { message } from '../api'
import { costDetails, exportStatistics, formatCosts, formatCount, formatMetric, metricDetails, quality, queryStatistics } from '../api/statistics'
import TablePagination from './TablePagination.vue'

const props = withDefaults(defineProps<{
  taskId: string
  dimension: Dimension
  parent?: Parent | null
  inspect?: boolean
}>(), { parent: null, inspect: true })
const emit = defineEmits<{ inspect: [row: StatRow], summary: [value: Totals] }>()
const state = reactive({ page: null as StatPage | null, draft: '', search: '', sort: 'cost' as StatSort,
  ascending: false, busy: false, exporting: false, error: '', expanded: [] as string[] })
let revision = 0
const sortOptions = [{ label: '金额 USD', value: 'cost' }, { label: '总 Token', value: 'tokens' }, { label: '请求次数', value: 'requests' }]
const columns: TableColumn<StatRow>[] = [
  { key: 'name', label: '名称', kind: 'custom', size: 'xl' },
  { key: 'requests', label: '请求数', kind: 'custom', size: 'md' },
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
    state.expanded = []
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
  return `成功 ${formatCount(value.succeeded)} · 失败 ${formatCount(value.failed)} · 取消 ${formatCount(value.cancelled)} · 未完成 ${formatCount(value.incomplete)}${value.unknownOutcomes !== '0' ? ` · 状态未细分 ${formatCount(value.unknownOutcomes)}` : ''}`
}
function toggleOutcomes(id: string) {
  state.expanded = state.expanded.includes(id) ? state.expanded.filter(key => key !== id) : [...state.expanded, id]
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
      <ZButton variant="text" :disabled="state.busy" @click="state.ascending = !state.ascending; load()">{{ state.ascending ? '升序' : '降序' }}</ZButton>
      <ZButton class="stat-export" :loading="state.exporting" :disabled="state.busy || !state.page?.total" @click="download">导出 CSV</ZButton>
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <p v-if="state.busy" class="muted" role="status">正在汇总分组</p>
    <p v-if="state.page?.observation.lag" class="notice">统计数据延迟 {{ state.page.observation.lag }} 秒</p>
    <ZTable :columns="columns" :data="state.page?.items ?? []" :expanded-row-keys="state.expanded" row-key="id" :empty-text="state.busy ? '正在读取' : '此范围没有匹配记录'">
      <template #name="{ row }">
        <div class="stat-name">
          <span>{{ row.name }}</span>
          <span v-if="quality(row.totals)" class="muted">{{ quality(row.totals) }}</span>
        </div>
      </template>
      <template #requests="{ row }">
        <ZButton variant="text" size="small" class="stat-request-trigger" :aria-expanded="state.expanded.includes(row.id)" :aria-label="`${row.name} 的 ${formatCount(row.totals.requests)} 次请求结果明细`" @click="toggleOutcomes(row.id)">
          <span aria-hidden="true">{{ state.expanded.includes(row.id) ? '▾' : '▸' }}</span>{{ formatCount(row.totals.requests) }}
        </ZButton>
      </template>
      <template #expanded="{ row }"><p class="stat-outcomes">{{ outcomes(row.totals) }}</p></template>
      <template #tokens="{ row }"><span class="stat-metric" :title="metricDetails(row.totals.tokens)">{{ formatMetric(row.totals.tokens) }}</span></template>
      <template #input="{ row }"><span class="stat-metric" :title="metricDetails(row.totals.input)">{{ formatMetric(row.totals.input) }}</span></template>
      <template #output="{ row }"><span class="stat-metric" :title="metricDetails(row.totals.output)">{{ formatMetric(row.totals.output) }}</span></template>
      <template #cacheRead="{ row }"><span class="stat-metric" :title="metricDetails(row.totals.cacheRead)">{{ formatMetric(row.totals.cacheRead) }}</span></template>
      <template #cacheWrite="{ row }"><span class="stat-metric" :title="metricDetails(row.totals.cacheWrite)">{{ formatMetric(row.totals.cacheWrite) }}</span></template>
      <template #cacheRatio="{ row }">{{ row.totals.cacheRatio === null ? '未知' : `${(row.totals.cacheRatio * 100).toFixed(1)}%` }}</template>
      <template #cost="{ row }"><span class="stat-metric" :title="costDetails(row.totals)">{{ formatCosts(row.totals.costs) }}</span></template>
      <template #actions="{ row }"><ZButton v-if="row.canInspect" variant="text" type="primary" @click="emit('inspect', row)">{{ dimension === 'key' ? '查看模型' : '查看 Key' }}</ZButton><span v-else class="muted">无分组维度</span></template>
    </ZTable>
    <TablePagination v-if="state.page" :offset="state.page.offset" :total="state.page.total" :page-size="state.page.pageSize" :busy="state.busy" @change="load" />
  </div>
</template>

<style scoped>
.stat-search { width: min(100%, 20rem); }
.stat-sort { width: min(100%, 10rem); }
.stat-export { margin-left: auto; }
.stat-name { display: grid; gap: 4px; overflow-wrap: anywhere; }
.stat-request-trigger { max-width: 100%; white-space: normal; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.stat-metric { display: block; min-width: 0; white-space: pre-line; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.stat-outcomes { margin: 0; padding: 12px 16px; white-space: normal; overflow-wrap: anywhere; color: var(--z-color-text-secondary); }
</style>
