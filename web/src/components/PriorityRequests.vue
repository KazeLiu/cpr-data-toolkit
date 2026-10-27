<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { PriorityAccount, PriorityLog } from '../api/model-priority'
import { ZButton, ZTable } from '@codex-proxy/ui'
import { reactive, watch } from 'vue'
import { message } from '../api'
import { clearPriorityLogs, getPriorityLogs } from '../api/model-priority'

const props = defineProps<{ active: boolean, accounts: PriorityAccount[] }>()
const logs = reactive({
  entries: [] as PriorityLog[],
  offset: 0,
  total: 0,
  pageSize: 25,
  capacity: 200,
  busy: false,
  loaded: false,
  error: '',
})
const kinds: Record<string, string> = {
  primary: '首选',
  backup: '备用',
  native_fallback: '其他备用交给宿主',
  unmatched: '未命中规则',
  rules_error: '规则读取失败',
}
const outcomes: Record<string, string> = {
  succeeded: '成功',
  failed: '失败',
  rejected: '已拒绝',
  cancelled: '已取消',
  incomplete: '未完成',
}
const columns: TableColumn<PriorityLog>[] = [
  { key: 'time', label: '时间', kind: 'custom', size: 'lg' },
  { key: 'model', label: '模型', kind: 'custom', size: 'xl' },
  { key: 'decisions', label: '插件选择', kind: 'custom', size: '2xl' },
  { key: 'actual', label: '最终上游', kind: 'custom', size: 'xl' },
  { key: 'result', label: '固定结果', kind: 'custom', size: 'lg' },
]
function name(id: string) {
  return props.accounts.find(account => account.id === id)?.name ?? id
}
function result(row: PriorityLog) {
  if (!row.completed)
    return '等待完成观测'
  if (row.sendState === 'not_sent')
    return '未发往上游'
  if (row.sendState === 'ambiguous')
    return '发送结果不确定'
  const last = row.decisions.at(-1)
  if (!last)
    return '未经过插件调度'
  if (last.selectedAccountId) {
    if (row.actualAccountId !== last.selectedAccountId)
      return '与插件选择不一致'
    return last.kind === 'primary' ? '首选匹配' : '备用匹配'
  }
  return kinds[last.kind] ?? last.kind
}
async function load(offset = 0) {
  logs.busy = true
  logs.error = ''
  try {
    const data = await getPriorityLogs(offset)
    logs.entries = data.entries
    logs.total = data.total
    logs.capacity = data.capacity
    logs.pageSize = data.pageSize
    logs.offset = offset
    logs.loaded = true
  }
  catch (error) {
    logs.error = message(error)
  }
  finally {
    logs.busy = false
  }
}
async function clear() {
  logs.busy = true
  logs.error = ''
  try {
    if (!(await clearPriorityLogs()))
      throw new Error('日志未清空，请重新读取')
    logs.entries = []
    logs.offset = 0
    logs.total = 0
  }
  catch (error) {
    logs.error = message(error)
  }
  finally {
    logs.busy = false
  }
}
watch(() => props.active, (active) => {
  if (active && !logs.loaded)
    void load()
}, { immediate: true })
</script>

<template>
  <section class="section">
    <div class="toolbar">
      <h2>最近请求</h2>
      <ZButton :loading="logs.busy" @click="load()">
        刷新日志
      </ZButton>
      <ZButton variant="text" :disabled="logs.busy || !logs.total" @click="clear">
        清空日志
      </ZButton>
    </div>
    <p class="muted">
      最终上游来自宿主完成事件，需启用 observer 的 request_completed 绑定 · 最近 {{ logs.capacity }} 条，重启后清空
    </p>
    <p v-if="logs.error" class="error" role="alert">
      {{ logs.error }}
    </p>
    <ZTable :columns="columns" :data="logs.entries" row-key="requestId" :loading="logs.busy" empty-text="暂无请求，请发送模型请求后刷新日志">
      <template #time="{ row }">
        <div class="request-cell">
          <time>{{ new Date(row.timeMs).toLocaleString() }}</time>
          <span class="muted mono" :title="row.requestId">{{ row.requestId }}</span>
        </div>
      </template>
      <template #model="{ row }">
        <div class="request-cell">
          <strong class="mono">{{ row.model ?? '未提供' }}</strong>
          <span class="muted">{{ row.provider }}</span>
        </div>
      </template>
      <template #decisions="{ row }">
        <div class="request-cell">
          <span v-if="!row.decisions.length" class="muted">未收到调度记录</span>
          <span v-for="decision in row.decisions" :key="decision.attemptIndex">
            {{ decision.attemptIndex }} · {{ kinds[decision.kind] ?? decision.kind }}
            <template v-if="decision.selectedAccountId"> {{ name(decision.selectedAccountId) }}</template>
            <span v-if="decision.primaryAccountId && decision.kind === 'backup'" class="muted">（首选 {{ name(decision.primaryAccountId) }}）</span>
          </span>
          <span v-if="row.decisionsTruncated" class="muted">仅保留最近 32 次选择</span>
        </div>
      </template>
      <template #actual="{ row }">
        <div class="request-cell">
          <strong :title="row.actualAccountId ?? ''">{{ row.actualAccountId ? name(row.actualAccountId) : row.completed ? '未选择上游' : '尚未确认' }}</strong>
          <span v-if="row.outcome" class="muted">{{ outcomes[row.outcome] ?? row.outcome }} · {{ row.attemptCount }} 次尝试</span>
        </div>
      </template>
      <template #result="{ row }">
        {{ result(row) }}
      </template>
    </ZTable>
    <div class="toolbar">
      <ZButton variant="text" :disabled="logs.busy || logs.offset === 0" @click="load(logs.offset - logs.pageSize)">
        上一页
      </ZButton>
      <ZButton variant="text" :disabled="logs.busy || logs.offset + logs.pageSize >= logs.total" @click="load(logs.offset + logs.pageSize)">
        下一页
      </ZButton>
      <span class="muted">{{ logs.entries.length ? logs.offset + 1 : 0 }}–{{ logs.offset + logs.entries.length }} / {{ logs.total }} 条</span>
    </div>
  </section>
</template>

<style scoped>
.request-cell { display: grid; gap: 6px; min-width: 0; white-space: normal; overflow-wrap: anywhere; }
.request-cell > .mono.muted { max-width: 14rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
