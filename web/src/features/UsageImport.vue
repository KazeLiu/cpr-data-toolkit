<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { UsageEvidence, UsageGroup, UsageOptions, UsageTask } from '../api'
import { ZButton, ZDatePicker, ZDialog, ZInput, ZSelect, ZTable } from '@codex-proxy/ui'
import { reactive, watch } from 'vue'
import { applyUsage, discardUsage, getUsage, message, prepareUsage } from '../api'

const props = defineProps<{ sourceId: string; locked: boolean }>()
const emit = defineEmits<{ busy: [value: boolean] }>()
const state = reactive({
  groups: [] as UsageGroup[],
  total: 0,
  offset: 0,
  detailRows: '',
  duplicates: '',
  rollups: '',
  warnings: [] as string[],
  totals: null as { requests: string | null; tokens: string | null; costUsd: string | null } | null,
  busy: false,
  error: '',
  notice: '',
  sourceName: 'cc-switch',
  timezone: 'Asia/Shanghai',
  app: 'codex',
  model: 'gpt',
  startTime: null as string | null,
  endTime: null as string | null,
  evidence: [] as UsageEvidence[],
  evidenceTotal: 0,
  reviewOffset: 0,
  reviewOpen: false,
  confirm: false,
  task: null as UsageTask | null,
  submitted: false,
  pause: false,
})
function sourceTime(milliseconds: number | null): string {
  if (milliseconds === null) return '未知'
  return new Intl.DateTimeFormat('zh-CN', {
    timeZone: state.task?.options.timezone ?? state.timezone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hourCycle: 'h23',
  }).format(new Date(milliseconds))
}
function sourceSeconds(seconds: string | null): string {
  if (seconds === null) return '未知'
  const value = Number(seconds)
  const date = new Date(value * 1000)
  return Number.isSafeInteger(value) && Number.isFinite(date.getTime())
    ? sourceTime(value * 1000)
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
      row.date ?? `${sourceSeconds(row.firstTimeSeconds)} 至 ${sourceSeconds(row.lastTimeSeconds)}`,
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
const appOptions = [
  { label: '全部应用', value: '' },
  { label: 'Codex', value: 'codex' },
  { label: 'Claude', value: 'claude' },
  { label: 'Claude Desktop', value: 'claude-desktop' },
  { label: 'Gemini', value: 'gemini' },
  { label: 'OpenCode', value: 'opencode' },
  { label: 'Grok Build', value: 'grokbuild' },
  { label: 'MCode', value: 'mcode' },
  { label: 'Pi', value: 'pi' },
]
function options(): UsageOptions {
  return {
    sourceId: props.sourceId,
    sourceName: state.sourceName,
    timezone: state.timezone,
    app: state.app || null,
    model: state.model,
    startTime: state.startTime || null,
    endTime: state.endTime || null,
  }
}
let revision = 0
async function load(offset = 0, reviewOffset = 0) {
  const id = props.sourceId
  if (!id) {
    state.groups = []
    state.total = 0
    state.offset = 0
    state.reviewOffset = 0
    state.detailRows = ''
    state.error = ''
    state.warnings = []
    return
  }
  state.busy = true
  state.error = ''
  state.notice = ''
  const current = revision
  try {
    const result = await getUsage(
      state.task?.options ?? options(),
      offset,
      state.task?.taskId ?? null,
      reviewOffset,
    )
    if (id !== props.sourceId || current !== revision) return false
    state.groups = result.groups
    state.total = result.total
    state.offset = offset
    state.detailRows = result.detailRows
    state.duplicates = result.excludedDuplicates
    state.rollups = result.rollupRows
    state.warnings = result.warnings
    state.totals = result.totals
    state.evidence = result.evidence
    state.evidenceTotal = result.evidenceTotal
    state.reviewOffset = reviewOffset
    return true
  } catch (error) {
    if (id === props.sourceId && current === revision) state.error = message(error)
    return false
  } finally {
    state.busy = false
  }
}
async function prepare() {
  state.busy = true
  emit('busy', true)
  state.error = ''
  state.notice = ''
  try {
    state.task = await prepareUsage(options())
    state.submitted = false
    if (!(await load())) return
    if (state.task.done) state.notice = '选定范围无需写入或清理，无重复累计'
    else state.confirm = true
  } catch (error) {
    state.error = message(error)
  } finally {
    state.busy = false
    emit('busy', false)
  }
}
async function run() {
  state.confirm = false
  state.busy = true
  state.submitted = true
  state.pause = false
  emit('busy', true)
  state.error = ''
  try {
    while (state.task && !state.task.done && !state.pause)
      state.task = await applyUsage(state.task.taskId, state.task.offset, state.task.options)
    state.notice = state.task?.done
      ? '用量已写入 CPR 原生统计，可到使用统计页查看'
      : '已暂停，可继续本次导入'
    if (state.task?.done) state.submitted = false
  } catch (error) {
    state.error = `${message(error)}；已停止后续批次，点击继续导入核对当前批次`
  } finally {
    state.busy = false
    emit('busy', !!state.task && !state.task.done)
  }
}
async function discard() {
  if (!state.task) return
  state.busy = true
  try {
    await discardUsage(state.task.taskId)
    state.task = null
    state.submitted = false
    state.notice = '任务已结束，已写入贡献保留，可按相同来源标识重新预览选定范围'
    state.error = ''
    emit('busy', false)
  } catch (error) {
    state.error = message(error)
  } finally {
    state.busy = false
  }
}
watch(
  () => [
    props.sourceId,
    state.sourceName,
    state.timezone,
    state.app,
    state.model,
    state.startTime,
    state.endTime,
  ],
  async (value, previous) => {
    revision++
    const old = state.task
    state.task = null
    state.submitted = false
    state.confirm = false
    state.reviewOpen = false
    state.groups = []
    state.total = 0
    state.offset = 0
    state.reviewOffset = 0
    state.detailRows = ''
    state.evidence = []
    state.evidenceTotal = 0
    state.warnings = []
    state.totals = null
    state.error = ''
    if (old) {
      state.busy = true
      try {
        await discardUsage(old.taskId)
      } catch (error) {
        state.error = message(error)
      } finally {
        state.busy = false
      }
    }
    if (!previous || value[0] !== previous[0]) await load()
    else state.notice = '筛选已修改，请重新预览'
  },
  { immediate: true },
)
</script>

<template>
  <section class="panel">
    <p class="muted">写入原生总量与趋势，不生成请求明细，不改变API密钥预算</p>
    <div class="form-grid">
      <label class="field"
        ><span>CCS 来源标识</span
        ><ZInput
          v-model="state.sourceName"
          maxlength="32"
          :disabled="state.busy || state.submitted"
      /></label>
      <label class="field"
        ><span>CCS 来源时区</span
        ><ZInput
          v-model="state.timezone"
          placeholder="例如 Asia/Shanghai"
          :disabled="state.busy || state.submitted"
      /></label>
    </div>
    <p class="muted">同一 CCS 安装始终使用相同来源标识，覆盖仅影响该来源，保留 CPR 原生请求和其他来源</p>
    <div class="form-grid">
      <label class="field"
        ><span>应用</span
        ><ZSelect
          v-model="state.app"
          :options="appOptions"
          :disabled="state.busy || state.submitted"
      /></label>
      <label class="field"
        ><span>模型名称</span
        ><ZInput
          v-model="state.model"
          placeholder="包含匹配，清空表示不限"
          :disabled="state.busy || state.submitted"
      /></label>
      <label class="field"
        ><span>开始时间</span
        ><ZDatePicker
          :model-value="state.startTime"
          type="datetime"
          format="YYYY-MM-DD HH:mm"
          value-format="YYYY-MM-DD[T]HH:mm"
          placeholder="不限开始时间"
          aria-label="开始时间"
          :disabled="state.busy || state.submitted"
          @update:model-value="state.startTime = $event as string | null"
      /></label>
      <label class="field"
        ><span>结束时间</span
        ><ZDatePicker
          :model-value="state.endTime"
          type="datetime"
          format="YYYY-MM-DD HH:mm"
          value-format="YYYY-MM-DD[T]HH:mm"
          placeholder="不限结束时间"
          aria-label="结束时间"
          :disabled="state.busy || state.submitted"
          @update:model-value="state.endTime = $event as string | null"
      /></label>
    </div>
    <p class="muted">
      时间留空表示不限，按 CCS 来源时区 {{ state.timezone }} 解释，包含开始时间、不包含结束时间 ·
      日聚合须完整覆盖来源日期
    </p>
    <p class="muted">
      覆盖会替换当前来源在所选时段的全部旧贡献，不限应用和模型，再导入筛选结果 · 旧贡献按归属时间判断
    </p>
    <div class="toolbar">
      <ZButton :disabled="!sourceId || state.busy || locked" @click="load(state.offset)">
        刷新筛选结果 </ZButton
      ><ZButton
        :type="state.task && !state.task.done ? undefined : 'primary'"
        :loading="state.busy"
        :disabled="!sourceId || locked || state.submitted || !state.timezone"
        @click="prepare"
      >
        预览导入变更
      </ZButton>
      <ZButton
        v-if="state.task && !state.task.done && !state.submitted"
        type="primary"
        :disabled="state.busy || locked"
        @click="state.confirm = true"
        >确认本次预览</ZButton
      >
      <ZButton
        v-if="state.submitted && state.task && !state.task.done"
        :disabled="state.busy"
        @click="run"
      >
        继续导入
      </ZButton>
      <ZButton
        v-if="state.submitted && state.task && !state.task.done"
        :disabled="state.busy"
        @click="discard"
        >结束本次任务</ZButton
      >
      <ZButton
        v-if="state.busy && state.submitted"
        :disabled="state.pause"
        @click="state.pause = true"
      >
        暂停后续批次
      </ZButton>
    </div>
    <p v-if="state.error" class="error" role="alert">
      {{ state.error }}
    </p>
    <p v-if="state.notice" class="notice" role="status">
      {{ state.notice }}
    </p>
    <p v-if="state.task" role="status">
      已确认 {{ state.task.offset }} / {{ state.task.total }} 项变更 · 来源贡献
      {{ state.task.imported }} 项 · 更新已有贡献 {{ state.task.updated }} 项 · 清理旧贡献
      {{ state.task.removed }} 项
    </p>
    <div v-if="state.task" class="metrics">
      <div>
        筛选后记录<strong>{{ state.task.filtered }}</strong>
      </div>
      <div>
        疑似重复排除<strong>{{ state.task.excluded }}</strong>
      </div>
      <div>
        无法确认<strong>{{ state.task.uncertain }}</strong>
      </div>
      <div>
        最终待导入<strong>{{ state.task.imported }}</strong>
      </div>
      <ZButton @click="state.reviewOpen = true">查看判重记录</ZButton>
    </div>
    <div v-if="state.detailRows" class="metrics">
      <div>
        有效明细<strong>{{ state.detailRows }}</strong>
      </div>
      <div>
        CCS 内部重复排除<strong>{{ state.duplicates }}</strong>
      </div>
      <div>
        日聚合行<strong>{{ state.rollups }}</strong>
      </div>
    </div>
    <div v-if="state.totals" class="metrics">
      <div>
        筛选后请求数<strong>{{ state.totals.requests ?? '未知' }}</strong>
      </div>
      <div>
        筛选后 Token<strong>{{ state.totals.tokens ?? '未知' }}</strong>
      </div>
      <div>
        筛选后来源费用 USD<strong>{{ state.totals.costUsd ?? '未知' }}</strong>
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
      :empty-text="
        !sourceId
          ? '选择来源后查看用量'
          : state.detailRows
            ? '没有符合筛选条件的用量'
            : '请刷新筛选结果或重新预览'
      "
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
      >
        下一页
      </ZButton>
    </div>
    <ZDialog v-model="state.confirm" title="是否覆盖所选时间段的 CCS 用量" type="warning">
      <div class="panel">
        <p>来源 {{ state.task?.options.sourceName }} · 时区 {{ state.task?.options.timezone }}</p>
        <p>
          {{ state.task?.options.startTime?.replace('T', ' ') ?? '不限开始时间' }} 至
          {{ state.task?.options.endTime?.replace('T', ' ') ?? '不限结束时间' }}
        </p>
        <p>替换该时段本来源的全部旧贡献，不限应用和模型，再导入本次筛选结果</p>
        <p class="muted">保留时段外数据、CPR 原生请求和其他来源</p>
      </div>
      <template #footer>
        <ZButton @click="state.confirm = false"> 取消 </ZButton
        ><ZButton type="primary" :disabled="state.busy || locked" @click="run"> 覆盖并导入 </ZButton>
      </template>
    </ZDialog>
    <ZDialog v-model="state.reviewOpen" title="CCS 与 CPR 请求核对">
      <section class="panel">
        <p class="muted">
          CCS 代理日志记录时间对 CPR 完成时间 ±30 秒 · 仅双方唯一对应时排除 · 时间按
          {{ state.task?.options.timezone }} 显示
        </p>
        <details
          v-for="(item, index) in state.evidence"
          :key="`${item.sourceId}:${index}`"
          class="group"
        >
          <summary>
            {{ item.excluded ? '排除' : '保留' }} · {{ item.model }} · {{ item.sourceId }}
          </summary>
          <p>
            {{ item.app }} / {{ item.provider }} ·
            {{ item.date ?? sourceTime(item.sourceTimeMs) }} ·
            {{ item.reason }}
          </p>
          <p>
            输入（含缓存）{{ item.input ?? '未知' }} · 输出 {{ item.output ?? '未知' }} · 缓存读
            {{ item.cacheRead ?? '未知' }} · 缓存写 {{ item.cacheWrite ?? '未知' }}
          </p>
          <div v-for="candidate in item.candidates" :key="candidate.id" class="panel">
            <p class="mono">CPR {{ candidate.id }}</p>
            <p>
              开始 {{ sourceTime(candidate.startedAtMs) }} · 完成
              {{ sourceTime(candidate.completedAtMs) }} · 时间差 {{ candidate.deltaMs / 1000 }} 秒
            </p>
            <p>
              输入（含缓存）{{ candidate.input }} · 输出 {{ candidate.output }} · 缓存读
              {{ candidate.cacheRead ?? '未知' }} · 缓存写 {{ candidate.cacheWrite ?? '未知' }}
            </p>
          </div>
          <p v-if="item.candidateCount > item.candidates.length" class="muted">
            共 {{ item.candidateCount }} 条候选，展示前 20 条，请缩小时间范围后核对
          </p>
        </details>
        <div class="toolbar">
          <ZButton
            :disabled="state.busy || state.reviewOffset === 0"
            @click="load(state.offset, state.reviewOffset - 100)"
            >上一页</ZButton
          >
          <span
            >{{ state.evidenceTotal ? state.reviewOffset + 1 : 0 }}–{{
              Math.min(state.reviewOffset + 100, state.evidenceTotal)
            }}
            / {{ state.evidenceTotal }}</span
          >
          <ZButton
            :disabled="state.busy || state.reviewOffset + 100 >= state.evidenceTotal"
            @click="load(state.offset, state.reviewOffset + 100)"
            >下一页</ZButton
          >
        </div>
      </section>
      <template #footer>
        <ZButton @click="state.reviewOpen = false">关闭</ZButton>
        <ZButton
          v-if="state.task && !state.task.done && !state.submitted"
          type="primary"
          :disabled="state.busy || locked"
          @click="
            state.reviewOpen = false;
            state.confirm = true
          "
          >确认本次预览</ZButton
        >
      </template>
    </ZDialog>
  </section>
</template>
