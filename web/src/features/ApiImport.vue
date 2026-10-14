<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, watch } from 'vue'
import { ZButton, ZDialog, ZIconButton, ZRadio, ZTable } from '@codex-proxy/ui'
import type { TableColumn } from '@codex-proxy/ui'
import { getImportKey, message, operationResults, previewImport, request } from '../api'
import type { Choice, ImportGroup, ImportSource, OperationResult } from '../api'
import ImportActionIcon from '../components/ImportActionIcon.vue'
import OperationResults from '../components/OperationResults.vue'

const props = defineProps<{ sourceId: string; locked: boolean; active: boolean }>()
const emit = defineEmits<{ busy: [value: boolean] }>()
const pageSize = 100
const state = reactive({
  busy: false,
  error: '',
  notice: '',
  note: '',
  rows: [] as ImportSource[],
  groups: [] as ImportGroup[],
  selected: {} as Record<string, boolean>,
  choices: {} as Record<string, string>,
  targets: {} as Record<string, string>,
  revealed: {} as Record<string, string>,
  keyBusy: {} as Record<string, boolean>,
  results: [] as OperationResult[],
  pendingIds: [] as string[],
  confirm: false,
  submitted: false,
  offset: 0,
})
let alive = true
let keyEpoch = 0
function clearKeys() {
  keyEpoch += 1
  state.revealed = {}
  state.keyBusy = {}
}
const disabled = computed(() => state.busy || props.locked || state.submitted)
const groupsById = computed(() => new Map(state.groups.map((group) => [group.id, group])))
const selectable = computed(() => state.rows.filter(canSelect))
const selectedRows = computed(() => selectable.value.filter((row) => state.selected[row.id]))
const selectedIds = computed(() => [...new Set(selectedRows.value.map((row) => row.groupId!))])
const allSelected = computed(
  () => selectable.value.length > 0 && selectedRows.value.length === selectable.value.length,
)
const partiallySelected = computed(() => selectedRows.value.length > 0 && !allSelected.value)
const visibleRows = computed(() => state.rows.slice(state.offset, state.offset + pageSize))
const pendingGroups = computed(() => state.pendingIds.map((id) => groupsById.value.get(id)!))
const promptGroups = computed(() =>
  pendingGroups.value.filter((group) => nameChoices(group).length > 1 || group.targets.length > 1),
)
const readyToSave = computed(
  () =>
    pendingGroups.value.length > 0 &&
    pendingGroups.value.every(
      (group) =>
        Boolean(state.choices[group.id]) &&
        (group.targets.length <= 1 || Boolean(state.targets[group.id])),
    ),
)
const columns: TableColumn<ImportSource>[] = [
  { key: 'select', label: '全选', width: 80 },
  { key: 'name', label: '来源名称', size: 'lg' },
  { key: 'appType', label: '应用', size: 'sm' },
  { key: 'address', label: 'API', size: '2xl' },
  { key: 'key', label: 'Key', size: 'xl' },
  { key: 'actions', label: '接入', size: 'lg' },
]
function appName(app: string) {
  return (
    ({ claude: 'Claude', codex: 'Codex', gemini: 'Gemini' } as Record<string, string>)[app] ?? app
  )
}
function canSelect(row: ImportSource) {
  return row.groupId !== null && groupsById.value.get(row.groupId)!.status !== 'unknown'
}
function selectAll(checked: boolean) {
  for (const row of selectable.value) state.selected[row.id] = checked
}
function nameChoices(group: ImportGroup): Choice[] {
  const names = new Set<string>()
  return group.choices.filter((choice) => {
    if (names.has(choice.name)) return false
    names.add(choice.name)
    return true
  })
}
function choiceApp(choice: Choice) {
  return choice.kind === 'host'
    ? '已有 CPR 账号'
    : appName(state.rows.find((row) => row.id === choice.id)!.appType)
}
async function load() {
  const id = props.sourceId
  state.rows = []
  state.groups = []
  clearKeys()
  state.selected = {}
  state.choices = {}
  state.targets = {}
  state.results = []
  state.pendingIds = []
  state.confirm = false
  state.submitted = false
  state.offset = 0
  state.error = ''
  state.notice = ''
  state.note = ''
  state.busy = false
  if (!id) return
  state.busy = true
  try {
    const result = await previewImport(id)
    if (id !== props.sourceId || !alive) return
    state.rows = result.rows
    state.groups = result.groups
    state.note = result.note
  } catch (error) {
    if (id === props.sourceId && alive) state.error = message(error)
  } finally {
    if (id === props.sourceId) state.busy = false
  }
}
function confirm(ids: string[]) {
  state.error = ''
  state.notice = ''
  state.pendingIds = ids
  for (const group of pendingGroups.value) {
    const choices = nameChoices(group)
    if (choices.length === 1) state.choices[group.id] = choices[0]!.id
    if (group.targets.length === 1) state.targets[group.id] = group.targets[0]!.accountId
  }
  state.confirm = true
}
async function save() {
  if (!readyToSave.value) return
  const sourceId = props.sourceId
  const selections = pendingGroups.value.map((group) => {
    const choice = group.choices.find((choice) => choice.id === state.choices[group.id])!
    const targetAccountId = state.targets[group.id] ?? null
    const target = group.targets.find((target) => target.accountId === targetAccountId)
    return {
      groupId: group.id,
      choiceId: choice.id,
      targetAccountId,
      expectedRevision: target?.revision ?? null,
    }
  })
  const labels = new Map(
    pendingGroups.value.map((group) => [
      group.id,
      group.choices.find((choice) => choice.id === state.choices[group.id])!.name,
    ]),
  )
  state.confirm = false
  state.busy = true
  state.error = ''
  state.results = []
  emit('busy', true)
  try {
    for (let offset = 0; offset < selections.length; offset += 100) {
      const batch = await request(
        'POST',
        'api/import/save',
        { sourceId, selections: selections.slice(offset, offset + 100) },
        operationResults,
      )
      const stopped = batch.some(
        (result) => result.status === 'unknown' || result.status === 'mapping_pending',
      )
      state.results.push(
        ...batch.map((result) => ({ ...result, label: labels.get(result.label)! })),
      )
      if (stopped) {
        state.results.push(
          ...selections.slice(offset + 100).map((selection) => ({
            label: labels.get(selection.groupId)!,
            status: 'not_run',
            message: '前项结果未知，本项未提交',
          })),
        )
        break
      }
    }
  } catch (error) {
    state.error = `${message(error)}，请重新预览核对账号，不要直接重复提交`
  } finally {
    state.submitted = true
    state.busy = false
    clearKeys()
    emit('busy', false)
  }
}
async function toggleKey(row: ImportSource) {
  if (state.revealed[row.id] !== undefined) {
    delete state.revealed[row.id]
    return
  }
  const sourceId = props.sourceId
  const epoch = keyEpoch
  state.keyBusy[row.id] = true
  state.error = ''
  try {
    const key = await getImportKey(sourceId, row.id)
    if (epoch === keyEpoch && props.active) state.revealed[row.id] = key
  } catch (error) {
    if (epoch === keyEpoch) state.error = message(error)
  } finally {
    if (epoch === keyEpoch) delete state.keyBusy[row.id]
  }
}
async function copy(value: string, label: string) {
  state.error = ''
  state.notice = ''
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(value)
    } else {
      const input = document.createElement('textarea')
      input.value = value
      input.style.position = 'fixed'
      input.style.opacity = '0'
      document.body.append(input)
      const focused = document.activeElement as HTMLElement | null
      try {
        input.select()
        if (!document.execCommand('copy')) throw new Error('copy')
      } finally {
        input.remove()
        focused?.focus()
      }
    }
    state.notice = `${label} 已复制`
  } catch {
    state.error = '复制失败，请检查浏览器剪贴板权限'
  }
}
async function copyKey(row: ImportSource) {
  const sourceId = props.sourceId
  const epoch = keyEpoch
  state.keyBusy[row.id] = true
  state.error = ''
  try {
    const key = state.revealed[row.id] ?? (await getImportKey(sourceId, row.id))
    if (epoch === keyEpoch && props.active) await copy(key, 'Key')
  } catch (error) {
    if (epoch === keyEpoch) state.error = message(error)
  } finally {
    if (epoch === keyEpoch) delete state.keyBusy[row.id]
  }
}
watch(() => props.sourceId, load, { immediate: true })
watch(
  () => props.active,
  (active) => {
    if (!active) clearKeys()
  },
)
onBeforeUnmount(() => {
  alive = false
  clearKeys()
})
</script>

<template>
  <section class="panel">
    <div class="toolbar">
      <ZButton :loading="state.busy" :disabled="!sourceId || locked || state.busy" @click="load"
        >重新预览</ZButton
      >
      <ZButton :disabled="disabled || !selectable.length" @click="selectAll(!allSelected)">{{
        allSelected ? '取消全选' : '全选'
      }}</ZButton>
      <ZButton
        type="primary"
        :disabled="disabled || !selectedIds.length"
        @click="confirm(selectedIds)"
        >一键接入 OpenAI Responses 兼容上游</ZButton
      >
      <span class="muted">已选 {{ selectedRows.length }} 项 · {{ selectedIds.length }} 个上游</span>
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <p v-if="state.notice" class="success" role="status">{{ state.notice }}</p>
    <p v-if="state.note" class="muted">{{ state.note }}</p>
    <div v-if="!sourceId" class="empty">先选择 CCS 完整 SQL 导出</div>
    <template v-else>
      <ZTable
        :columns="columns"
        :data="visibleRows"
        row-key="id"
        :loading="state.busy"
        size="small"
        empty-text="未找到来源配置"
      >
        <template #header-select>
          <label class="table-select-all">
            <input
              type="checkbox"
              class="native-check"
              :checked="allSelected"
              :indeterminate="partiallySelected"
              :disabled="disabled || !selectable.length"
              aria-label="全选所有可接入来源"
              @change="selectAll(($event.target as HTMLInputElement).checked)"
            />
            <span>全选</span>
          </label>
        </template>
        <template #select="{ row }"
          ><input
            v-model="state.selected[row.id]"
            type="checkbox"
            class="native-check"
            :disabled="disabled || !canSelect(row)"
            :aria-label="`选择 ${row.name}`"
        /></template>
        <template #name="{ row }"
          ><span class="import-cell">{{ row.name }}</span></template
        >
        <template #appType="{ row }">{{ appName(row.appType) }}</template>
        <template #address="{ row }">
          <div class="import-secret-cell">
            <span class="mono import-cell">{{ row.address ?? '未识别' }}</span>
            <ZIconButton
              v-if="row.address"
              size="small"
              :disabled="state.busy || locked"
              :aria-label="`复制 ${row.name} 的 API 地址`"
              @click="copy(row.address, 'API')"
              ><ImportActionIcon name="copy"
            /></ZIconButton>
          </div>
        </template>
        <template #key="{ row }">
          <div class="import-secret-cell">
            <span class="mono import-cell">{{
              row.hasKey ? (state.revealed[row.id] ?? '••••••••') : '未识别'
            }}</span>
            <div v-if="row.hasKey" class="import-secret-actions">
              <ZIconButton
                size="small"
                :disabled="state.busy || locked || state.keyBusy[row.id]"
                :aria-label="`${state.revealed[row.id] === undefined ? '查看' : '隐藏'} ${row.name} 的 Key`"
                :pressed="state.revealed[row.id] !== undefined"
                @click="toggleKey(row)"
                ><ImportActionIcon :name="state.revealed[row.id] === undefined ? 'eye' : 'eye-off'"
              /></ZIconButton>
              <ZIconButton
                size="small"
                :loading="state.keyBusy[row.id]"
                :disabled="state.busy || locked"
                :aria-label="`复制 ${row.name} 的 Key`"
                @click="copyKey(row)"
                ><ImportActionIcon name="copy"
              /></ZIconButton>
            </div>
          </div>
        </template>
        <template #actions="{ row }">
          <div class="import-cell">
            <ZButton
              v-if="canSelect(row)"
              variant="text"
              size="small"
              :disabled="disabled"
              @click="confirm([row.groupId!])"
              >接入</ZButton
            >
            <span v-else class="muted">{{
              row.reason ?? groupsById.get(row.groupId!)?.message
            }}</span>
          </div>
        </template>
      </ZTable>
      <div v-if="state.rows.length > pageSize" class="toolbar">
        <ZButton
          variant="text"
          :disabled="state.offset === 0 || state.busy"
          @click="state.offset -= pageSize"
          >上一页</ZButton
        >
        <ZButton
          variant="text"
          :disabled="state.offset + pageSize >= state.rows.length || state.busy"
          @click="state.offset += pageSize"
          >下一页</ZButton
        >
        <span class="muted"
          >{{ state.offset + 1 }}–{{ state.offset + visibleRows.length }} /
          {{ state.rows.length }} 项 · 全选包含所有页</span
        >
      </div>
    </template>
    <OperationResults :results="state.results" />
    <ZDialog
      v-model="state.confirm"
      title="接入 OpenAI Responses 兼容上游"
      type="warning"
      :width="720"
    >
      <div class="panel">
        <p>将接入 {{ pendingGroups.length }} 个上游，相同 API 与 Key 合并为一项</p>
        <p class="muted">新建项进入 CPR 账号管理，已有项按选择更新名称和连接配置</p>
        <fieldset v-for="group in promptGroups" :key="group.id" class="group">
          <legend class="mono">{{ group.address }}</legend>
          <div v-if="nameChoices(group).length > 1" class="field">
            <strong>请选择接入名称</strong>
            <ZRadio
              v-for="choice in nameChoices(group)"
              :key="choice.id"
              class="import-name-option"
              :model-value="state.choices[group.id] ?? ''"
              @update:model-value="state.choices[group.id] = String($event)"
              :name="`name-${group.id}`"
              :value="choice.id"
              :aria-label="`使用名称 ${choice.name}`"
            >
              <span
                >{{ choice.name }} <span class="muted">· {{ choiceApp(choice) }}</span></span
              >
            </ZRadio>
          </div>
          <div v-if="group.targets.length > 1" class="field">
            <strong>请选择要更新的 CPR 账号</strong>
            <ZRadio
              v-for="target in group.targets"
              :key="target.accountId"
              class="import-name-option"
              :model-value="state.targets[group.id] ?? ''"
              @update:model-value="state.targets[group.id] = String($event)"
              :name="`target-${group.id}`"
              :value="target.accountId"
              :aria-label="`更新账号 ${target.accountId}`"
            >
              <span
                >{{ group.choices.find((choice) => choice.accountId === target.accountId)!.name }}
                <span class="muted mono">· {{ target.accountId }}</span></span
              >
            </ZRadio>
          </div>
        </fieldset>
        <p v-if="!readyToSave" class="notice">请为重复连接选择名称，并选定需要更新的账号</p>
      </div>
      <template #footer
        ><ZButton @click="state.confirm = false">取消</ZButton
        ><ZButton type="primary" :disabled="!readyToSave || state.busy || locked" @click="save"
          >确认接入</ZButton
        ></template
      >
    </ZDialog>
  </section>
</template>
