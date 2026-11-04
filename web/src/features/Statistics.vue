<script setup lang="ts">
import type { Dimension, Parent, Preset, StatRow, StatTask, Totals } from '../api/statistics'
import { ZButton, ZDatePicker, ZDialog, ZSelect } from '@codex-proxy/ui'
import { computed, onBeforeUnmount, reactive, watch } from 'vue'
import { ApiError, message } from '../api'
import { cancelStatistics, formatCosts, formatCount, formatMetric, getStatisticsMetadata, quality, startStatistics, stepStatistics } from '../api/statistics'
import StatisticsTable from '../components/StatisticsTable.vue'

const props = defineProps<{ active: boolean }>()
const state = reactive({ timezone: Intl.DateTimeFormat().resolvedOptions().timeZone, timezones: [] as string[],
  preset: '7d', presets: [] as Preset[], range: null as string[] | null, metadataBusy: false,
  task: null as StatTask | null, running: false, inFlight: false, starting: false, blocked: false, error: '', formError: '',
  summary: null as Totals | null, details: null as { title: string, dimension: Dimension, parent: Parent } | null })
let revision = 0
let disposed = false
const dimensions: { id: Dimension, title: string }[] = [{ id: 'model', title: '按模型统计' }, { id: 'key', title: '按 Key 统计' }, { id: 'group', title: '按当前 Key 分组统计' }]
const timezoneOptions = computed(() => state.timezones.map(value => ({ value, label: value })))
const presetOptions = computed(() => [...state.presets.map(preset => ({ value: preset.id, label: preset.name })), { value: 'custom', label: '自定义范围' }])
const phase = computed(() => state.task ? ({ keys: '读取当前 Key 分组', records: '读取历史请求', aggregates: '读取聚合贡献', complete: '统计完成' }[state.task.phase]) : '')
function selectPreset() {
  const preset = state.presets.find(preset => preset.id === state.preset)
  if (preset) state.range = [...preset.range]
}
async function metadata() {
  state.metadataBusy = true
  state.formError = ''
  try {
    const result = await getStatisticsMetadata(state.timezone)
    if (disposed) return
    state.timezones = result.timezones
    state.presets = result.presets
    selectPreset()
  }
  catch (error) { state.formError = message(error) }
  finally { state.metadataBusy = false }
}
async function run() {
  if (!state.task || state.inFlight || state.task.phase === 'complete' || state.blocked) return
  const current = revision
  state.running = true
  state.inFlight = true
  state.error = ''
  try {
    while (state.running && props.active && !disposed && current === revision) {
      const result = await stepStatistics(state.task.taskId, state.task.step)
      if (current !== revision || disposed) return
      state.task = result
      if (result.phase === 'complete') break
    }
  }
  catch (error) {
    if (current === revision) {
      state.error = message(error)
      state.blocked = error instanceof ApiError && (error.status === 400 || error.status === 404)
    }
  }
  finally { if (current === revision) { state.running = false; state.inFlight = false } }
}
async function start() {
  if (!state.range || state.range.length !== 2 || state.starting || state.inFlight) return
  state.starting = true
  state.error = ''
  try {
    const task = await startStatistics(state.timezone, state.range[0], state.range[1])
    const previous = state.task
    revision += 1
    if (disposed) { await cancelStatistics(task.taskId); return }
    state.task = task
    state.summary = null
    state.details = null
    state.blocked = false
    if (previous) await cancelStatistics(previous.taskId)
    void run()
  }
  catch (error) { state.error = message(error) }
  finally { state.starting = false }
}
async function cancel() {
  const task = state.task
  revision += 1
  state.running = false
  state.inFlight = false
  state.task = null
  state.summary = null
  state.details = null
  state.error = ''
  if (task) {
    try { await cancelStatistics(task.taskId) }
    catch (error) { state.error = message(error) }
  }
}
function inspect(dimension: Dimension, row: StatRow) {
  state.details = { title: row.name, parent: { dimension, id: row.id }, dimension: dimension === 'key' ? 'model' : 'key' }
}
watch(() => props.active, (active) => {
  if (!active) { state.running = false; state.details = null }
  else if (!state.presets.length && !state.metadataBusy) void metadata()
}, { immediate: true })
onBeforeUnmount(() => {
  disposed = true
  revision += 1
  state.running = false
  if (state.task) void cancelStatistics(state.task.taskId).catch(() => { /* 页面已关闭，任务由后端闲置期限清理 */ })
})
</script>

<template>
  <div class="panel">
    <section class="section">
      <h2>统计范围</h2>
      <div class="stat-filters">
        <label class="field"><span>统计时区</span><ZSelect v-model="state.timezone" :options="timezoneOptions" filterable :disabled="state.running || state.starting || state.metadataBusy" @change="metadata" /></label>
        <label class="field"><span>快捷范围</span><ZSelect v-model="state.preset" :options="presetOptions" :disabled="state.running || state.starting || state.metadataBusy" @change="selectPreset" /></label>
        <label class="field stat-range"><span>日期与时间</span><ZDatePicker
          :model-value="state.range" type="datetimerange" format="YYYY-MM-DD HH:mm" value-format="YYYY-MM-DD[T]HH:mm"
          start-placeholder="开始时间" end-placeholder="结束时间" aria-label="统计日期与分钟范围"
          :disabled="state.running || state.starting || state.metadataBusy"
          @update:model-value="state.range = $event as string[] | null" @change="state.preset = 'custom'"
        /></label>
      </div>
      <div class="toolbar">
        <ZButton type="primary" :loading="state.starting" :disabled="state.inFlight || state.metadataBusy || !state.range || state.range.length !== 2" @click="start">查询统计</ZButton>
        <ZButton v-if="state.running" @click="state.running = false">暂停</ZButton>
        <ZButton v-if="state.task && state.task.phase !== 'complete' && !state.running && !state.blocked" :disabled="state.starting || state.inFlight" @click="run">继续查询</ZButton>
        <ZButton v-if="state.task" :disabled="state.starting" @click="cancel">取消并清理</ZButton>
        <ZButton v-if="state.formError" :loading="state.metadataBusy" @click="metadata">重新读取时间范围</ZButton>
      </div>
      <p v-if="state.formError" class="error" role="alert">{{ state.formError }}</p>
      <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
      <p class="muted">按所选时区解释，包含开始、不包含结束 · 结束时间晚于当前时刻时截到查询开始时刻</p>
      <p v-if="state.task" class="muted" role="status">
        {{ phase }}{{ !state.running && state.task.phase !== 'complete' ? ' · 已暂停' : '' }} · 已读 {{ state.task.scanned }} 条请求、{{ state.task.contributions }} 项聚合贡献
        <br>{{ state.task.startDisplay }} 至 {{ state.task.endDisplay }}（{{ state.task.timezone }}）
      </p>
    </section>
    <template v-if="state.task?.phase === 'complete'">
      <section v-if="state.summary" class="stat-overview" aria-label="所选范围汇总">
        <div><span class="muted">总请求</span><strong>{{ formatCount(state.summary.requests) }}</strong></div>
        <div><span class="muted">总 Token</span><strong>{{ formatMetric(state.summary.tokens) }}</strong></div>
        <div><span class="muted">缓存读取</span><strong>{{ formatMetric(state.summary.cacheRead) }}</strong></div>
        <div><span class="muted">缓存写入</span><strong>{{ formatMetric(state.summary.cacheWrite) }}</strong></div>
        <div><span class="muted">已知费用</span><strong>{{ formatCosts(state.summary.costs) }}</strong></div>
        <p v-if="quality(state.summary)" class="muted stat-overview-note">{{ quality(state.summary) }}</p>
      </section>
      <p class="muted">输入已包含缓存读写，总 Token 不重复累加缓存 · Token 和费用统计成功请求及外部聚合贡献，金额保留历史记录，不代表供应商账单</p>
      <section v-for="dimension in dimensions" :key="dimension.id" class="section">
        <h2>{{ dimension.title }}</h2>
        <p v-if="dimension.id === 'group'" class="muted">按本次扫描读取的 Key 当前绑定关系归类，不代表请求时的历史分组 · 多组绑定分别计入各组，组间合计不能相加</p>
        <StatisticsTable :task-id="state.task.taskId" :dimension="dimension.id" @inspect="inspect(dimension.id, $event)" @summary="dimension.id === 'model' && (state.summary = $event)" />
      </section>
      <p class="muted">仅覆盖宿主仍保留的明细与聚合贡献 · 分页期间历史修改或清理可能影响结果 · 扫描不是跨调用一致快照，任务闲置 15 分钟后清理</p>
    </template>
    <p v-else-if="!state.task" class="muted">选择时间范围后查询，读取完成后展示全部分组，不限于前 100 项</p>
    <ZDialog :model-value="state.details !== null" :title="state.details?.title ?? '交叉明细'" width="min(94vw, 90rem)" @update:model-value="!$event && (state.details = null)">
      <StatisticsTable v-if="state.details && state.task" :key="`${state.details.parent.dimension}:${state.details.parent.id}`" :task-id="state.task.taskId" :dimension="state.details.dimension" :parent="state.details.parent" :inspect="false" />
    </ZDialog>
  </div>
</template>

<style scoped>
.stat-filters { display: grid; grid-template-columns: minmax(12rem, 1fr) minmax(9rem, 0.7fr) minmax(22rem, 2fr); align-items: end; gap: 12px; }
.stat-overview { display: grid; grid-template-columns: repeat(5, minmax(0, 1fr)); gap: 16px; padding: 16px; border-radius: var(--z-border-radius); background: var(--z-color-fill-quaternary); }
.stat-overview > div { display: grid; gap: 6px; min-width: 0; }
.stat-overview strong { font-size: var(--z-font-size-lg); font-variant-numeric: tabular-nums; overflow-wrap: anywhere; }
.stat-overview-note { grid-column: 1 / -1; margin: 0; }
@container toolkit (max-width: 900px) { .stat-filters { grid-template-columns: repeat(2, minmax(0, 1fr)); } .stat-range { grid-column: 1 / -1; } .stat-overview { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
@container toolkit (max-width: 480px) { .stat-filters { grid-template-columns: minmax(0, 1fr); } }
</style>
