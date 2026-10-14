<script setup lang="ts">
import { reactive, watch } from 'vue'
import { ZButton, ZTable } from '@codex-proxy/ui'
import type { TableColumn } from '@codex-proxy/ui'
import { getUsage, message } from '../api'
import type { UsageGroup } from '../api'
const props = defineProps<{ sourceId: string }>()
const state = reactive({
  groups: [] as UsageGroup[],
  total: 0,
  offset: 0,
  detailRows: '',
  duplicates: '',
  rollups: '',
  warnings: [] as string[],
  busy: false,
  error: '',
  notice: '',
})
function utc(seconds: string | null): string {
  if (seconds === null) return '未知'
  const value = Number(seconds)
  const date = new Date(value * 1000)
  return Number.isSafeInteger(value) && Number.isFinite(date.getTime())
    ? date.toISOString()
    : `UTC 秒 ${seconds}`
}
const columns: TableColumn<UsageGroup>[] = [
  {
    key: 'granularity',
    label: '粒度',
    size: 'sm',
    formatter: (value) => (value === 'daily' ? '日聚合' : '请求明细汇总'),
  },
  { key: 'appType', label: '应用', size: 'sm' },
  { key: 'providerId', label: '来源供应商', kind: 'mono', size: 'lg' },
  { key: 'model', label: '模型', kind: 'mono', size: 'lg' },
  { key: 'sourceContext', label: '来源口径', kind: 'custom', size: 'xl' },
  {
    key: 'time',
    label: '来源时间',
    size: 'xl',
    formatter: (_, row) =>
      row.date ?? `${utc(row.firstTimeSeconds)} 至 ${utc(row.lastTimeSeconds)}`,
  },
  { key: 'requests', label: '请求数', kind: 'numeric' },
  { key: 'inputTokens', label: '新增输入', kind: 'numeric', emptyText: '未知' },
  { key: 'outputTokens', label: '输出', kind: 'numeric', emptyText: '未知' },
  {
    key: 'cacheReadTokens',
    label: '缓存读',
    kind: 'numeric',
    emptyText: '未知',
  },
  {
    key: 'cacheWriteTokens',
    label: '缓存写',
    kind: 'numeric',
    emptyText: '未知',
  },
  { key: 'totalTokens', label: '总 Token', kind: 'numeric', emptyText: '未知' },
  {
    key: 'originalCostUsd',
    label: '来源费用 USD',
    kind: 'numeric',
    emptyText: '未知',
  },
]
async function load(offset = 0) {
  const id = props.sourceId
  if (!id) {
    state.groups = []
    state.total = 0
    state.detailRows = ''
    state.error = ''
    state.warnings = []
    return
  }
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    const result = await getUsage(id, offset)
    if (id !== props.sourceId) return
    state.groups = result.groups
    state.total = result.total
    state.offset = offset
    state.detailRows = result.detailRows
    state.duplicates = result.excludedDuplicates
    state.rollups = result.rollupRows
    state.warnings = result.warnings
  } catch (error) {
    if (id === props.sourceId) state.error = message(error)
  } finally {
    state.busy = false
  }
}
watch(
  () => props.sourceId,
  () => load(),
  { immediate: true },
)
</script>

<template>
  <section class="panel">
    <p class="notice">上游接口未开放 · 当前仅解析和预览，不能补入 CPR 原生统计</p>
    <div class="toolbar">
      <ZButton :loading="state.busy" :disabled="!sourceId" @click="load(state.offset)"
        >刷新预览</ZButton
      ><ZButton :disabled="!sourceId || state.busy" @click="state.notice = '上游接口未开放'"
        >确认导入</ZButton
      >
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <p v-if="state.notice" class="notice" role="status">{{ state.notice }}</p>
    <div v-if="state.detailRows" class="metrics">
      <div>
        有效明细<strong>{{ state.detailRows }}</strong>
      </div>
      <div>
        跨源重复排除<strong>{{ state.duplicates }}</strong>
      </div>
      <div>
        日聚合行<strong>{{ state.rollups }}</strong>
      </div>
    </div>
    <p v-for="warning in state.warnings" :key="warning" class="muted">
      {{ warning }}
    </p>
    <ZTable
      :columns="columns"
      :data="state.groups"
      :loading="state.busy"
      size="small"
      empty-text="选择来源后查看用量，未知字段保持未知"
    >
      <template #sourceContext="{ row }">
        <details>
          <summary>查看来源</summary>
          <dl>
            <dt>请求模型</dt>
            <dd class="mono">{{ row.requestModel ?? '未知' }}</dd>
            <dt>计价模型</dt>
            <dd class="mono">{{ row.pricingModel ?? '未知' }}</dd>
            <dt>数据来源</dt>
            <dd>{{ row.dataSources.map((source) => source ?? '未知').join('、') }}</dd>
            <dt>原始输入 Token</dt>
            <dd class="mono">{{ row.rawInputTokens ?? '未知' }}</dd>
            <dt>缓存语义</dt>
            <dd>
              {{
                row.inputTokenSemantics
                  .map(
                    (value) => ['旧版', '总输入含缓存读写', '新增输入'][value] ?? `未知 ${value}`,
                  )
                  .join('、')
              }}
            </dd>
          </dl>
        </details>
      </template>
    </ZTable>
    <div v-if="state.total" class="toolbar">
      <ZButton :disabled="state.busy || state.offset === 0" @click="load(state.offset - 100)"
        >上一页</ZButton
      ><span
        >{{ state.offset + 1 }}–{{ Math.min(state.offset + 100, state.total) }} /
        {{ state.total }} 个分组</span
      ><ZButton
        :disabled="state.busy || state.offset + 100 >= state.total"
        @click="load(state.offset + 100)"
        >下一页</ZButton
      >
    </div>
  </section>
</template>
