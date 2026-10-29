<script setup lang="ts">
import type { TableColumn } from '@codex-proxy/ui'
import type { PriorityAccount, PriorityRule } from '../api/model-priority'
import { ZButton, ZDialog, ZInput, ZScrollbar, ZSelect, ZSwitch, ZTable } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { message } from '../api'
import {
  deletePriority,
  getPriorities,
  lookupPriorityAccounts,
  savePriority,
} from '../api/model-priority'
import PriorityRequests from '../components/PriorityRequests.vue'

const props = defineProps<{ active: boolean }>()
const state = reactive({
  rules: [] as PriorityRule[],
  accounts: [] as PriorityAccount[],
  version: null as number | null,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
})
const editor = reactive({
  open: false,
  existing: false,
  provider: 'openai',
  model: '',
  enabled: true,
  accountIds: [] as string[],
  matched: null as string[] | null,
  failures: [] as { id: string, name: string, message: string }[],
  busy: false,
  error: '',
  dragId: '',
  overId: '',
})
const removal = reactive({ rule: null as PriorityRule | null, open: false, error: '' })
const providers = computed(() =>
  [
    ...new Set([
      ...state.accounts.map(account => account.provider),
      ...state.rules.map(rule => rule.provider),
    ]),
  ].sort(),
)
const directory = computed(() => new Map(state.accounts.map(account => [account.id, account])))
const columns: TableColumn<PriorityRule>[] = [
  { key: 'provider', label: 'Provider', size: 'sm' },
  { key: 'model', label: '模型名称', kind: 'mono', size: 'xl' },
  { key: 'order', label: '上游顺序', kind: 'custom', size: '3xl' },
  { key: 'enabled', label: '状态', kind: 'custom', size: 'sm' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'lg' },
]

function name(id: string) {
  return directory.value.get(id)?.name ?? id
}

async function load() {
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    const result = await getPriorities()
    state.version = result.version
    state.rules = result.rules
    state.accounts = result.accounts
    state.loaded = true
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}

function edit(rule?: PriorityRule) {
  editor.existing = !!rule
  editor.provider
    = rule?.provider
      ?? (providers.value.includes('openai') ? 'openai' : (providers.value[0] ?? 'openai'))
  editor.model = rule?.model ?? ''
  editor.enabled = rule?.enabled ?? true
  editor.accountIds = [...(rule?.accountIds ?? [])]
  editor.matched = null
  editor.failures = []
  editor.error = ''
  editor.dragId = ''
  editor.overId = ''
  editor.open = true
}

async function lookup() {
  editor.model = editor.model.trim()
  if (!editor.model)
    return
  if (
    !editor.existing
    && state.rules.some(rule => rule.provider === editor.provider && rule.model === editor.model)
  ) {
    editor.error = '该模型已有规则，请在规则列表中编辑'
    return
  }
  editor.busy = true
  editor.error = ''
  editor.failures = []
  try {
    const result = await lookupPriorityAccounts(editor.provider, editor.model)
    editor.matched = result.accounts.map(account => account.id)
    const refreshed = new Map(state.accounts.map(account => [account.id, account]))
    for (const account of result.accounts) refreshed.set(account.id, account)
    state.accounts = [...refreshed.values()]
    // 目录查询失败时保留已有顺序
    const ids = editor.existing ? [...editor.accountIds] : []
    const seen = new Set(ids)
    editor.accountIds = [...ids, ...editor.matched.filter(id => !seen.has(id))]
    editor.failures = result.failures
  }
  catch (error) {
    editor.error = message(error)
  }
  finally {
    editor.busy = false
  }
}

function move(index: number, target: number) {
  if (target < 0 || target >= editor.accountIds.length)
    return
  const [id] = editor.accountIds.splice(index, 1)
  editor.accountIds.splice(target, 0, id!)
}

function startDrag(event: DragEvent, id: string) {
  if (!event.dataTransfer)
    return
  editor.dragId = id
  event.dataTransfer.effectAllowed = 'move'
  event.dataTransfer.setData('text/plain', id)
}

function endDrag() {
  editor.dragId = ''
  editor.overId = ''
}

function drop(id: string) {
  if (!editor.dragId || editor.busy)
    return
  move(editor.accountIds.indexOf(editor.dragId), editor.accountIds.indexOf(id))
  endDrag()
}

async function save() {
  editor.busy = true
  editor.error = ''
  state.notice = ''
  try {
    const result = await savePriority(state.version, {
      provider: editor.provider,
      model: editor.model.trim(),
      enabled: editor.enabled,
      accountIds: [...editor.accountIds],
    })
    state.version = result.version
    state.rules = result.rules
    editor.open = false
    state.notice = '规则已保存，匹配账号调度绑定的新请求按最新顺序选择上游'
  }
  catch (error) {
    editor.error = message(error)
  }
  finally {
    editor.busy = false
  }
}

async function remove() {
  if (!removal.rule)
    return
  state.busy = true
  removal.error = ''
  state.notice = ''
  try {
    const result = await deletePriority(state.version, removal.rule.provider, removal.rule.model)
    state.version = result.version
    state.rules = result.rules
    removal.open = false
    state.notice = '规则已删除，此模型恢复宿主原有调度'
  }
  catch (error) {
    removal.error = message(error)
  }
  finally {
    state.busy = false
  }
}

function confirmRemoval(rule: PriorityRule) {
  removal.rule = rule
  removal.error = ''
  removal.open = true
}

watch(
  [() => editor.provider, () => editor.model],
  () => {
    if (!editor.existing)
      editor.accountIds = []
    editor.matched = null
    editor.failures = []
    editor.error = ''
  },
  { flush: 'sync' },
)
watch(
  () => props.active,
  (active) => {
    if (active && !state.loaded)
      void load()
  },
  { immediate: true },
)
</script>

<template>
  <div class="panel">
    <section class="section">
      <div class="toolbar">
        <ZButton
          type="primary"
          :disabled="!state.loaded || state.busy || !state.accounts.length"
          @click="edit()"
        >
          新增规则
        </ZButton>
        <ZButton :loading="state.busy" @click="load">
          重新读取
        </ZButton>
      </div>
      <p class="notice">
        保存规则后，在宿主为本插件启用 scheduler（账号调度）绑定，并选择需要生效的API密钥范围
      </p>
      <p v-if="state.error" class="error" role="alert">
        {{ state.error }}
      </p>
      <p v-if="state.notice" class="muted" role="status">
        {{ state.notice }}
      </p>
      <p v-if="state.loaded && !state.accounts.length" class="muted">
        暂无宿主账号，请先在「账号管理」添加上游
      </p>
      <ZTable
        :columns="columns"
        :data="state.rules"
        :loading="state.busy"
        empty-text="暂无规则，新增后可按模型设置首选和备用上游"
      >
        <template #order="{ row }">
          <ol class="priority-summary">
            <li v-for="(id, index) in row.accountIds" :key="id">
              <span class="muted">{{ index === 0 ? '首选' : `备用 ${index}` }}</span> {{ name(id) }}
            </li>
          </ol>
        </template>
        <template #enabled="{ row }">
          {{ row.enabled ? '已启用' : '已停用' }}
        </template>
        <template #actions="{ row }">
          <ZButton size="small" variant="text" :disabled="state.busy" @click="edit(row)">
            编辑
          </ZButton>
          <ZButton
            size="small"
            type="danger"
            variant="text"
            :disabled="state.busy"
            @click="confirmRemoval(row)"
          >
            删除
          </ZButton>
        </template>
      </ZTable>
      <p class="muted">
        默认将其他可用上游作为备用；如不希望某个上游承接此模型，请在「账号管理」中将该模型加入黑名单
      </p>
    </section>

    <PriorityRequests :active="active" :accounts="state.accounts" />

    <ZDialog
      v-model="editor.open"
      :title="editor.existing ? '编辑模型上游优先级' : '新增模型上游优先级'"
      width="48rem"
      :close-on-click-modal="!editor.busy"
      :close-on-press-escape="!editor.busy"
      :show-close="!editor.busy"
    >
      <div class="panel">
        <div class="form-grid">
          <label class="field"><span>Provider</span><ZSelect
            :model-value="editor.provider"
            :options="providers.map((provider) => ({ value: provider, label: provider }))"
            :disabled="editor.existing || editor.busy"
            placeholder="选择 Provider"
            @update:model-value="editor.provider = $event ?? ''"
          /></label>
          <label class="field"><span>模型名称</span><ZInput
            v-model="editor.model"
            :disabled="editor.existing || editor.busy"
            placeholder="填写完整上游模型 ID，精确匹配"
            @keydown.enter="!editor.busy && lookup()"
          /></label>
        </div>
        <div class="toolbar">
          <ZButton
            :loading="editor.busy"
            :disabled="!editor.provider || !editor.model.trim()"
            @click="lookup"
          >
            获取同名模型上游
          </ZButton>
          <ZSwitch v-model="editor.enabled" :disabled="editor.busy" label="启用规则" />
        </div>
        <p v-if="editor.error" class="error" role="alert">
          {{ editor.error }}
        </p>
        <details v-if="editor.failures.length" class="priority-failures">
          <summary>{{ editor.failures.length }} 个上游的模型目录未能读取，当前结果不完整</summary>
          <ZScrollbar max-height="8rem">
            <ul>
              <li v-for="failure in editor.failures" :key="failure.id">
                {{ failure.name }}：{{ failure.message }}
              </li>
            </ul>
          </ZScrollbar>
        </details>
        <ZScrollbar v-if="editor.accountIds.length" max-height="22rem">
          <ol class="priority-order" aria-label="上游优先顺序">
            <li
              v-for="(id, index) in editor.accountIds"
              :key="id"
              :draggable="!editor.busy"
              class="priority-row"
              :class="{
                'priority-dragging': editor.dragId === id,
                'priority-drop': editor.overId === id && editor.dragId !== id,
              }"
              @dragstart="startDrag($event, id)"
              @dragover.prevent="editor.overId = id"
              @drop.prevent="drop(id)"
              @dragend="endDrag"
            >
              <svg
                class="priority-handle"
                width="18"
                height="24"
                viewBox="0 0 18 24"
                fill="currentColor"
                aria-hidden="true"
              >
                <circle
                  v-for="(point, i) in [
                    [6, 6],
                    [12, 6],
                    [6, 12],
                    [12, 12],
                    [6, 18],
                    [12, 18],
                  ]"
                  :key="i"
                  :cx="point[0]"
                  :cy="point[1]"
                  r="1.5"
                />
              </svg>
              <div class="priority-account">
                <strong :title="id">{{ name(id) }}</strong>
                <span class="muted">{{ index === 0 ? '首选上游' : `备用 ${index}`
                }}<template v-if="!directory.has(id)"> · 账号已删除</template><template v-else-if="!directory.get(id)!.enabled"> · 账号已停用</template><template v-if="editor.matched && !editor.matched.includes(id)">
                  · 目录未匹配</template></span>
              </div>
              <div class="priority-move">
                <ZButton
                  size="small"
                  variant="text"
                  :disabled="editor.busy || index === 0"
                  :aria-label="`将 ${name(id)} 上移`"
                  @click="move(index, index - 1)"
                >
                  上移
                </ZButton>
                <ZButton
                  size="small"
                  variant="text"
                  :disabled="editor.busy || index === editor.accountIds.length - 1"
                  :aria-label="`将 ${name(id)} 下移`"
                  @click="move(index, index + 1)"
                >
                  下移
                </ZButton>
              </div>
            </li>
          </ol>
        </ZScrollbar>
        <p v-else class="muted">
          {{
            editor.matched === null
              ? '填写模型名称后获取上游'
              : '没有找到包含该模型的上游，请先在账号管理核对模型目录'
          }}
        </p>
      </div>
      <template #footer>
        <ZButton :disabled="editor.busy" @click="editor.open = false">
          取消
        </ZButton>
        <ZButton
          type="primary"
          :loading="editor.busy"
          :disabled="!editor.accountIds.length || !editor.model.trim()"
          @click="save"
        >
          保存规则
        </ZButton>
      </template>
    </ZDialog>

    <ZDialog
      v-model="removal.open"
      title="删除优先级规则"
      :close-on-click-modal="!state.busy"
      :close-on-press-escape="!state.busy"
      :show-close="!state.busy"
    >
      <p>
        删除 {{ removal.rule?.provider }} / {{ removal.rule?.model }} 的规则后，恢复宿主原有调度
      </p>
      <p v-if="removal.error" class="error" role="alert">
        {{ removal.error }}
      </p>
      <template #footer>
        <ZButton :disabled="state.busy" @click="removal.open = false">
          取消
        </ZButton><ZButton type="danger" :loading="state.busy" @click="remove">
          删除规则
        </ZButton>
      </template>
    </ZDialog>
  </div>
</template>

<style scoped>
.priority-summary {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 16px;
  max-height: 5rem;
  overflow: auto;
  margin: 0;
  padding: 0;
  list-style: none;
}
.priority-summary li {
  overflow-wrap: anywhere;
}
.priority-order {
  display: grid;
  gap: 8px;
  margin: 0;
  padding: 2px;
  list-style: none;
}
.priority-row {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  border: 1px solid var(--cp-color-border-secondary);
  border-radius: var(--cp-border-radius);
  background: var(--cp-color-bg-container);
}
.priority-handle {
  flex-shrink: 0;
  color: var(--cp-color-text-tertiary);
  cursor: grab;
}
.priority-account {
  display: grid;
  flex: 1;
  min-width: 0;
  gap: 4px;
  overflow-wrap: anywhere;
}
.priority-move {
  display: flex;
  flex-wrap: wrap;
  flex-shrink: 0;
  gap: 4px;
}
.priority-dragging {
  opacity: 0.5;
}
.priority-drop {
  border-color: var(--cp-color-primary);
  background: var(--cp-color-primary-container);
}
.priority-failures {
  color: var(--cp-color-warning-text);
}
.priority-failures summary {
  cursor: pointer;
}
@media (max-width: 560px) {
  .priority-row {
    gap: 8px;
    padding: 10px 8px;
  }
  .priority-move {
    flex-direction: column;
  }
}
</style>
