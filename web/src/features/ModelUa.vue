<script setup lang="ts">
import { computed, onMounted, reactive, watch } from 'vue'
import { ZButton, ZDialog, ZInput, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import type { TableColumn } from '@codex-proxy/ui'
import {
  clearUaLogs,
  diagnose,
  getKeys,
  getModels,
  getRules,
  getUaLogs,
  message,
  previewRules,
  saveRules,
} from '../api'
import type { Header, Key, Rule, UaLogStatus, UaRequestLog } from '../api'

const rules = reactive({
  rows: [] as Rule[],
  version: null as number | null,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
  saved: '',
})
const editor = reactive({ open: false, index: -1, value: emptyRule() })
const probe = reactive({
  model: '',
  headers: [] as Header[],
  ruleId: null as string | null,
  evaluated: false,
  busy: false,
  error: '',
})
const diagnostic = reactive({
  keys: [] as Key[],
  key: '',
  models: [] as string[],
  model: '',
  baselineUa: '',
  busy: false,
  error: '',
  result: null as Awaited<ReturnType<typeof diagnose>> | null,
})
const logs = reactive({
  entries: [] as UaRequestLog[],
  offset: 0,
  total: 0,
  capacity: 200,
  pageSize: 25,
  loaded: false,
  busy: false,
  error: '',
})
const logColumns: TableColumn<UaRequestLog>[] = [
  { key: 'timeMs', label: '时间', size: 'lg', kind: 'custom' },
  { key: 'model', label: '模型名称', size: 'lg', kind: 'custom' },
  { key: 'rule', label: '规则', size: 'lg', kind: 'custom' },
  { key: 'before', label: '替换前 UA', size: '3xl', kind: 'custom' },
  { key: 'after', label: '替换后 UA', size: '3xl', kind: 'custom' },
]
const logLabels: Record<UaLogStatus, string> = {
  rewritten: '已改写',
  unmatched: '未命中',
  missing_model: '模型缺失，未改写',
  rules_error: '规则读取失败，未改写',
}
function hasExtraChanges(entry: UaRequestLog) {
  return (
    entry.before.originator !== entry.after.originator || entry.before.version !== entry.after.version
  )
}
function headerValue(value: string | null) {
  return value === null ? '未提供' : value || '空值'
}
async function loadLogs(offset = 0) {
  logs.busy = true
  logs.error = ''
  try {
    const result = await getUaLogs(offset)
    logs.entries = result.entries
    logs.total = result.total
    logs.capacity = result.capacity
    logs.pageSize = result.pageSize
    logs.offset = offset
    logs.loaded = true
  } catch (error) {
    logs.error = message(error)
  } finally {
    logs.busy = false
  }
}
async function clearLogs() {
  logs.busy = true
  logs.error = ''
  try {
    if (!(await clearUaLogs())) throw new Error('日志未清空，请刷新后确认')
    logs.entries = []
    logs.total = 0
    logs.offset = 0
    logs.loaded = true
  } catch (error) {
    logs.error = message(error)
  } finally {
    logs.busy = false
  }
}
const dirty = computed(() => rules.loaded && rules.saved !== JSON.stringify(rules.rows))
const columns: TableColumn<Rule>[] = [
  { key: 'enabled', label: '启用', size: 'xs', kind: 'custom' },
  { key: 'keyword', label: '模型包含', size: 'md' },
  { key: 'userAgent', label: 'User-Agent', size: '3xl', kind: 'mono' },
  { key: 'actions', label: '操作', size: 'xl', kind: 'actions' },
]
function emptyRule(): Rule {
  return {
    id: `${Date.now()}-${Math.random().toString(36).slice(2)}`,
    enabled: true,
    keyword: '',
    userAgent: '',
    originator: null,
    version: null,
  }
}
async function load() {
  rules.busy = true
  rules.error = ''
  rules.notice = ''
  try {
    const result = await getRules()
    rules.rows = result.rules
    rules.version = result.version
    rules.loaded = true
    rules.saved = JSON.stringify(rules.rows)
  } catch (error) {
    rules.error = message(error)
  } finally {
    rules.busy = false
  }
}
const reload = reactive({ open: false })
async function reloadRules() {
  reload.open = false
  await load()
}
function edit(index: number) {
  editor.index = index
  editor.value = index < 0 ? emptyRule() : { ...rules.rows[index]! }
  editor.open = true
}
function applyEdit() {
  if (!editor.value.keyword.trim() || !editor.value.userAgent.trim()) {
    rules.error = '模型关键词和 UA 不能为空'
    return
  }
  const value = {
    ...editor.value,
    keyword: editor.value.keyword.trim(),
    originator: editor.value.originator?.trim() || null,
    version: editor.value.version?.trim() || null,
  }
  if (editor.index < 0) rules.rows.push(value)
  else rules.rows[editor.index] = value
  editor.open = false
  rules.error = ''
}
function move(index: number, delta: number) {
  const target = index + delta
  if (target >= 0 && target < rules.rows.length)
    [rules.rows[index], rules.rows[target]] = [rules.rows[target]!, rules.rows[index]!]
}
async function save() {
  rules.busy = true
  rules.error = ''
  rules.notice = ''
  try {
    rules.version = await saveRules(rules.version, rules.rows)
    rules.saved = JSON.stringify(rules.rows)
    rules.notice = '规则已保存，新请求使用最新规则'
  } catch (error) {
    rules.error = message(error)
  } finally {
    rules.busy = false
  }
}
async function preview() {
  probe.busy = true
  probe.error = ''
  probe.evaluated = false
  const model = probe.model
  const snapshot = JSON.stringify(rules.rows)
  try {
    const result = await previewRules(model, rules.rows)
    if (model !== probe.model || snapshot !== JSON.stringify(rules.rows)) return
    probe.headers = result.headers
    probe.ruleId = result.ruleId
    probe.evaluated = true
  } catch (error) {
    probe.error = message(error)
  } finally {
    probe.busy = false
  }
}
watch(
  () => [probe.model, JSON.stringify(rules.rows)],
  () => {
    probe.evaluated = false
  },
)
watch(
  () => [diagnostic.model, diagnostic.baselineUa],
  () => {
    diagnostic.result = null
  },
)
async function models() {
  diagnostic.model = ''
  diagnostic.models = []
  diagnostic.result = null
  diagnostic.error = ''
  diagnostic.busy = true
  try {
    diagnostic.models = await getModels(diagnostic.key)
  } catch (error) {
    diagnostic.error = message(error)
  } finally {
    diagnostic.busy = false
  }
}
async function runDiagnostic() {
  diagnostic.busy = true
  diagnostic.error = ''
  diagnostic.result = null
  try {
    diagnostic.result = await diagnose(
      diagnostic.key,
      diagnostic.model,
      diagnostic.baselineUa || null,
    )
  } catch (error) {
    diagnostic.error = message(error)
  } finally {
    diagnostic.busy = false
  }
}
onMounted(async () => {
  void loadLogs()
  await load()
  try {
    diagnostic.keys = await getKeys()
  } catch (error) {
    diagnostic.error = message(error)
  }
})
</script>

<template>
  <section class="panel">
    <p class="muted">
      忽略大小写，按顺序使用首条命中规则 · 未命中时保留API密钥身份 · 作用范围沿用宿主 request
      绑定，空绑定范围表示全局
    </p>
    <div class="toolbar">
      <ZButton :disabled="!rules.loaded || rules.busy || rules.rows.length >= 100" @click="edit(-1)"
        >新增规则</ZButton
      >
      <ZButton
        type="primary"
        :loading="rules.busy"
        :disabled="!rules.loaded || !dirty"
        @click="save"
        >保存规则</ZButton
      >
      <ZButton variant="text" :disabled="rules.busy" @click="dirty ? (reload.open = true) : load()"
        >重新读取</ZButton
      >
      <span v-if="dirty" class="muted">有未保存修改</span>
    </div>
    <p v-if="rules.error" class="error" role="alert">{{ rules.error }}</p>
    <p v-if="rules.notice" class="success" role="status">{{ rules.notice }}</p>
    <ZTable
      :columns="columns"
      :data="rules.rows"
      row-key="id"
      size="small"
      empty-text="暂无规则，新增后保存即可生效"
    >
      <template #enabled="{ row }"
        ><ZSwitch v-model="row.enabled" :disabled="rules.busy" :aria-label="`启用 ${row.keyword}`"
      /></template>
      <template #actions="{ row, index }"
        ><div class="actions">
          <ZButton size="small" variant="text" :disabled="rules.busy" @click="edit(index)"
            >编辑</ZButton
          ><ZButton
            size="small"
            variant="text"
            :disabled="rules.busy || index === 0"
            :aria-label="`上移 ${row.keyword}`"
            @click="move(index, -1)"
            >上移</ZButton
          ><ZButton
            size="small"
            variant="text"
            :disabled="rules.busy || index === rules.rows.length - 1"
            :aria-label="`下移 ${row.keyword}`"
            @click="move(index, 1)"
            >下移</ZButton
          ><ZButton
            size="small"
            type="danger"
            variant="text"
            :disabled="rules.busy"
            @click="rules.rows.splice(index, 1)"
            >删除</ZButton
          >
        </div></template
      >
    </ZTable>
    <div class="section">
      <div class="toolbar">
        <h2>请求改写日志</h2>
        <ZButton :loading="logs.busy" @click="loadLogs()">刷新日志</ZButton>
        <ZButton variant="text" :disabled="logs.busy || !logs.total" @click="clearLogs"
          >清空日志</ZButton
        >
        <span class="muted">最近 {{ logs.capacity }} 条 · 最新在前 · 重启插件后清空</span>
      </div>
      <p class="muted">
        替换前为插件入口 UA，不是API密钥默认 UA · 替换后为插件交给宿主的 UA · 预览与安全验证不计入日志
      </p>
      <p v-if="logs.error" class="error" role="alert">{{ logs.error }}</p>
      <ZTable
        :columns="logColumns"
        :data="logs.entries"
        :loading="logs.busy"
        row-key="id"
        size="small"
        :empty-text="
          logs.loaded
            ? '暂无检测记录 · 发送模型请求后刷新 · 仍为空时检查插件启用状态与 request 绑定范围'
            : '读取日志后查看实际请求'
        "
      >
        <template #timeMs="{ row }">
          <time :datetime="new Date(row.timeMs).toISOString()" class="ua-log-time">
            {{ new Date(row.timeMs).toLocaleString('zh-CN', { hour12: false }) }}
          </time>
        </template>
        <template #model="{ row }">
          <div class="ua-log-cell">
            <span class="mono">{{ row.model ?? '未提供模型名' }}</span>
            <details class="ua-log-details">
              <summary>请求 ID</summary>
              <span class="mono">{{ row.requestId }}</span>
            </details>
          </div>
        </template>
        <template #rule="{ row }">
          <div class="ua-log-cell">
            <span
              class="ua-log-status"
              :class="row.status === 'rewritten' ? 'success' : 'error'"
              >{{ logLabels[row.status] }}</span
            >
            <span v-if="row.rule" class="mono" :title="`规则 ID：${row.rule.id}`">
              包含 {{ row.rule.keyword }}
            </span>
            <span v-if="row.nextFailed" class="error">下游调用失败</span>
          </div>
        </template>
        <template #before="{ row }">
          <div class="ua-log-cell">
            <span class="mono ua-log-header">{{ headerValue(row.before.userAgent) }}</span>
            <details v-if="hasExtraChanges(row)" class="ua-log-details">
              <summary>配套头</summary>
              <dl>
                <dt>originator</dt>
                <dd class="mono">{{ headerValue(row.before.originator) }}</dd>
                <dt>version</dt>
                <dd class="mono">{{ headerValue(row.before.version) }}</dd>
              </dl>
            </details>
          </div>
        </template>
        <template #after="{ row }">
          <div class="ua-log-cell">
            <span class="mono ua-log-header">{{ headerValue(row.after.userAgent) }}</span>
            <details v-if="hasExtraChanges(row)" class="ua-log-details">
              <summary>配套头</summary>
              <dl>
                <dt>originator</dt>
                <dd class="mono">{{ headerValue(row.after.originator) }}</dd>
                <dt>version</dt>
                <dd class="mono">{{ headerValue(row.after.version) }}</dd>
              </dl>
            </details>
          </div>
        </template>
      </ZTable>
      <div class="toolbar">
        <ZButton
          variant="text"
          :disabled="logs.busy || logs.offset === 0"
          @click="loadLogs(logs.offset - logs.pageSize)"
          >上一页</ZButton
        >
        <ZButton
          variant="text"
          :disabled="logs.busy || logs.offset + logs.pageSize >= logs.total"
          @click="loadLogs(logs.offset + logs.pageSize)"
          >下一页</ZButton
        >
        <span class="muted">
          {{ logs.entries.length ? logs.offset + 1 : 0 }}–{{ logs.offset + logs.entries.length }}
          / {{ logs.total }} 条
        </span>
      </div>
    </div>
    <div class="section">
      <h2>匹配预览</h2>
      <div class="toolbar">
        <ZInput
          v-model="probe.model"
          aria-label="预览模型名"
          placeholder="例如 grok-4.7"
          style="max-width: 360px"
        /><ZButton :loading="probe.busy" :disabled="!probe.model" @click="preview"
          >预览当前编辑</ZButton
        >
        <div
          v-if="probe.evaluated"
          class="match-result"
          :class="probe.ruleId ? 'success' : 'error'"
          role="status"
        >
          <template v-if="probe.ruleId">
            <strong>已命中</strong>
            <span v-for="header in probe.headers" :key="header.name" class="mono"
              >{{ header.name }}: {{ header.value }}</span
            >
          </template>
          <strong v-else>未命中</strong>
        </div>
      </div>
      <p v-if="probe.error" class="error" role="alert">{{ probe.error }}</p>
    </div>
    <div class="section">
      <h2>安全验证</h2>
      <p class="notice">使用已保存规则 · 验证到插件出口 · 不发送上游推理请求</p>
      <div class="form-grid">
        <label class="field"
          ><span>API密钥</span
          ><ZSelect
            :model-value="diagnostic.key"
            @update:model-value="diagnostic.key = $event ?? ''"
            :options="
              diagnostic.keys.filter((k) => k.enabled).map((k) => ({ label: k.name, value: k.id }))
            "
            :disabled="diagnostic.busy"
            placeholder="选择API密钥"
            @change="models"
        /></label>
        <label class="field"
          ><span>模型</span
          ><ZSelect
            :model-value="diagnostic.model"
            @update:model-value="diagnostic.model = $event ?? ''"
            :options="diagnostic.models.map((m) => ({ label: m, value: m }))"
            :disabled="diagnostic.busy || !diagnostic.key"
            filterable
            placeholder="从该API密钥可见模型中选择"
        /></label>
        <label class="field"
          ><span>模拟入口 UA，可选</span
          ><ZInput
            v-model="diagnostic.baselineUa"
            :disabled="diagnostic.busy"
            placeholder="留空时不推测API密钥默认 UA"
        /></label>
      </div>
      <div class="toolbar">
        <ZButton
          :loading="diagnostic.busy"
          :disabled="!diagnostic.key || !diagnostic.model"
          @click="runDiagnostic"
          >验证已保存规则</ZButton
        >
      </div>
      <p v-if="diagnostic.error" class="error" role="alert">
        {{ diagnostic.error }}
      </p>
      <div
        v-if="diagnostic.result"
        class="diagnostic-result"
        :class="diagnostic.result.ruleId ? 'success' : 'error'"
        role="status"
      >
        <strong>
          {{ diagnostic.result.ruleId ? '已命中规则，插件出口头已改写' : '未命中规则，插件未改写' }}
        </strong>
        <dl v-if="diagnostic.result.headers.length">
          <template v-for="header in diagnostic.result.headers" :key="header.name"
            ><dt>{{ header.name }}</dt>
            <dd class="mono">{{ header.value }}</dd></template
          >
        </dl>
        <details class="diagnostic-details">
          <summary>验证范围</summary>
          <ol class="compact-list">
            <li v-for="stage in diagnostic.result.stages" :key="stage">{{ stage }}</li>
          </ol>
          <p>
            未覆盖：{{ diagnostic.result.notCovered.join('、')
            }}{{ !diagnostic.result.baselineKnown ? ' · 未读取真实API密钥默认 UA' : '' }}
          </p>
        </details>
      </div>
    </div>
    <ZDialog
      v-model="editor.open"
      :title="editor.index < 0 ? '新增模型 UA 规则' : '编辑模型 UA 规则'"
    >
      <div class="panel">
        <label class="field"
          ><span>模型包含</span
          ><ZInput v-model="editor.value.keyword" maxlength="256" placeholder="例如 grok" /></label
        ><label class="field"
          ><span>User-Agent</span><ZInput v-model="editor.value.userAgent" maxlength="4096"
        /></label>
        <details>
          <summary>配套身份头，可选</summary>
          <div class="panel" style="margin-top: 12px">
            <label class="field"
              ><span>originator</span
              ><ZInput
                :model-value="editor.value.originator ?? ''"
                @update:model-value="editor.value.originator = $event" /></label
            ><label class="field"
              ><span>version</span
              ><ZInput
                :model-value="editor.value.version ?? ''"
                @update:model-value="editor.value.version = $event" /></label
            ><span class="muted">未填写的头保持原样</span>
          </div>
        </details>
        <p v-if="rules.error" class="error" role="alert">{{ rules.error }}</p>
      </div>
      <template #footer
        ><ZButton variant="text" @click="editor.open = false">取消</ZButton
        ><ZButton type="primary" @click="applyEdit">加入编辑列表</ZButton></template
      >
    </ZDialog>
    <ZDialog v-model="reload.open" title="重新读取规则" type="warning"
      ><p>重新读取将放弃当前未保存编辑</p>
      <template #footer
        ><ZButton @click="reload.open = false">取消</ZButton
        ><ZButton type="primary" @click="reloadRules">确认重新读取</ZButton></template
      ></ZDialog
    >
  </section>
</template>
