<script setup lang="ts">
import type { QueryType, QuotaSource, QuotaSupport, UpstreamState } from '../api/upstream'
import { ZButton, ZSelect, ZSwitch } from '@codex-proxy/ui'
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { ApiError, message } from '../api'
import {
  getQuotaSources,
  getQuotaSupport,
  getUpstream,
  queryOptions,
  refreshUpstream,
  saveUpstream,
} from '../api/upstream'
import QuotaSharing from '../components/QuotaSharing.vue'
import UpstreamQuotaCards from '../components/UpstreamQuotaCards.vue'

const props = defineProps<{ active: boolean }>()
const support = ref<QuotaSupport | null>(null)
const state = reactive({
  sources: [] as QuotaSource[],
  accountId: '',
  queryType: 'disabled' as QueryType,
  snapshots: {} as Record<string, UpstreamState>,
  errors: {} as Record<string, string>,
  aggregate: true,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
})
const data = computed(() => state.snapshots[state.accountId])
const dirty = computed(() => data.value !== undefined && state.queryType !== data.value.queryType)
const source = computed(() => state.sources.find(source => source.id === state.accountId))
const sourceOptions = computed(() => state.sources.map(source => ({
  value: source.id,
  label: `${source.name}${source.enabled ? '' : '（已停用）'}`,
})))
const now = ref(Date.now())
let disposed = false
let refreshTimer: ReturnType<typeof setTimeout> | undefined
let cooldownTimer: ReturnType<typeof setTimeout> | undefined
const viewSources = computed(() => state.aggregate
  ? state.sources.filter(source => source.enabled)
  : source.value ? [source.value] : [])
const cards = computed(() => viewSources.value.flatMap((source) => {
  const value = state.snapshots[source.id]
  if (!value || (state.aggregate && value.queryType === 'disabled'))
    return []
  return (value.snapshot?.accounts ?? []).map(account => ({
    source,
    account,
    stale: value.stale,
  }))
}))
const issues = computed(() => viewSources.value.flatMap((source) => {
  const error = state.errors[source.id] ?? state.snapshots[source.id]?.error
  return error ? [{ id: source.id, name: source.name, error }] : []
}))
const blockedSources = computed(() => viewSources.value.filter(source => state.snapshots[source.id]?.forwardingBlocked))
const selectedQuery = computed(() => queryOptions.find(option => option.value === state.queryType)!)
const queryFields = computed(() => selectedQuery.value.path.map((value, level) => ({
  level,
  value,
  label: ['查询平台', '查询产品', '查询地区'][level],
  options: [...new Set(queryOptions
    .filter(option => selectedQuery.value.path.slice(0, level).every((part, index) => option.path[index] === part))
    .map(option => option.path[level])
    .filter((part): part is string => part !== undefined))]
    .map(part => ({ value: part, label: part })),
})))

function selectQueryLevel(level: number, value: string | undefined) {
  if (value === undefined)
    return
  const prefix = [...selectedQuery.value.path.slice(0, level), value]
  const matches = queryOptions.filter(option => prefix.every((part, index) => option.path[index] === part))
  state.queryType = (matches.find(option => selectedQuery.value.path.slice(level + 1)
    .every((part, index) => option.path[level + 1 + index] === part)) ?? matches[0]).value
}
function cooldownUntil(value: UpstreamState) {
  const interval = ['kimi_coding', 'kimi_coding_global', 'command_code'].includes(value.queryType) ? 15000 : 10000
  return value.lastAttemptAt === null ? 0 : Date.parse(value.lastAttemptAt) + interval
}
const canRefresh = computed(() => support.value?.queryAvailable && !dirty.value && !state.busy && viewSources.value.some((source) => {
  const value = state.snapshots[source.id]
  return source.enabled && (!value || (value.queryType !== 'disabled' && cooldownUntil(value) <= now.value))
}))
const resetHint = computed(() =>
  state.queryType === 'cpr'
    ? '上游 CPR 需为此 API 秘钥开启额度分享，名称和模型按上游分享设置显示'
    : state.queryType.startsWith('kimi')
      ? 'Kimi 查询使用所选地区的官方额度接口'
      : state.queryType === 'cline_pass'
        ? '使用账号中的 Cline API Key 查询官方套餐额度，不可使用 CPA 或 CPR 的客户端 Key'
        : state.queryType === 'command_code'
          ? 'Command Code 查询使用官方控制面接口，沿用账号当前凭据和出站代理'
          : '使用上游账号的 API 地址和当前凭据查询，不自动探测上游类型',
)

function isActive() {
  return props.active && !disposed
}

function stopTimers() {
  clearTimeout(refreshTimer)
  clearTimeout(cooldownTimer)
}

function syncCooldown() {
  clearTimeout(cooldownTimer)
  now.value = Date.now()
  if (!isActive())
    return
  const next = viewSources.value
    .flatMap(source => state.snapshots[source.id] ? [cooldownUntil(state.snapshots[source.id])] : [])
    .filter(time => time > now.value)
  if (next.length)
    cooldownTimer = setTimeout(syncCooldown, Math.min(...next) - now.value)
}

function schedule() {
  clearTimeout(refreshTimer)
  syncCooldown()
  if (!disposed) {
    refreshTimer = setTimeout(() => {
      // 可见性只决定是否跳过本轮，不在切回页面时补发请求
      if (document.visibilityState === 'visible')
        void load()
      else schedule()
    }, 180000)
  }
}

function apply(accountId: string, value: UpstreamState) {
  state.snapshots[accountId] = value
  if (accountId === state.accountId)
    state.queryType = value.queryType
}

async function fetchSource(source: QuotaSource, refresh: boolean) {
  if (!isActive())
    return
  delete state.errors[source.id]
  try {
    let value = await getUpstream(source.id)
    apply(source.id, value)
    if (refresh && support.value?.queryAvailable && isActive() && source.enabled && value.queryType !== 'disabled' && cooldownUntil(value) <= Date.now()) {
      value = await refreshUpstream(source.id)
      apply(source.id, value)
    }
  }
  catch (error) {
    if (error instanceof ApiError && error.status === 409)
      delete state.snapshots[source.id]
    state.errors[source.id] = message(error)
  }
}

async function read() {
  if (!source.value || state.busy)
    return
  state.queryType = data.value?.queryType ?? 'disabled'
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    await fetchSource(source.value, false)
  }
  finally {
    state.busy = false
    syncCooldown()
  }
}

async function load() {
  if (!isActive() || state.busy || dirty.value) {
    schedule()
    return
  }
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    support.value = await getQuotaSupport()
    state.sources = await getQuotaSources()
    state.loaded = true
    if (!state.sources.some(source => source.id === state.accountId)) {
      state.accountId
        = state.sources.find(source => source.enabled)?.id ?? state.sources[0]?.id ?? ''
    }
    const targets = [...viewSources.value]
    const selected = source.value
    if (selected && !targets.some(target => target.id === selected.id))
      targets.push(selected)
    let cursor = 0
    // 聚合时限制并发，离开页面后不再启动后续账号查询
    await Promise.all(Array.from({ length: Math.min(4, targets.length) }, async () => {
      while (isActive() && cursor < targets.length) {
        const target = targets[cursor++]
        await fetchSource(target, true)
      }
    }))
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
    schedule()
  }
}

async function save() {
  if (!data.value || !source.value || state.busy || !support.value?.queryAvailable)
    return
  const selected = source.value
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    apply(selected.id, await saveUpstream(selected.id, data.value.version, state.queryType))
    delete state.errors[selected.id]
    state.notice = state.snapshots[selected.id]?.error
      ? '设置已保存，本次获取失败'
      : state.queryType === 'disabled'
        ? '已关闭查询'
        : '设置已保存，额度已获取，插件将继续定期刷新'
  }
  catch (error) {
    if (error instanceof ApiError && error.status === 409) {
      delete state.snapshots[selected.id]
      await fetchSource(selected, false)
      state.error = '查询设置已被更新，请核对最新设置后再保存'
    }
    else {
      state.error = message(error)
    }
  }
  finally {
    state.busy = false
    schedule()
  }
}

async function refresh() {
  if (!canRefresh.value)
    return
  await load()
}

onMounted(() => void load())
onBeforeUnmount(() => {
  disposed = true
  stopTimers()
})
</script>

<template>
  <div class="panel">
    <section v-if="support && (!support.queryAvailable || !support.sharingAvailable)" class="section" role="status">
      <h3>宿主额度接口尚未就绪</h3>
      <p class="notice">
        {{ !support.queryAvailable && !support.sharingAvailable ? '当前宿主尚不支持额度查询和分享' : !support.queryAvailable ? '当前宿主尚不支持额度查询' : '当前宿主尚不支持额度分享' }}
        ，请使用包含 <a href="https://github.com/zyycn/codex-proxy-rs/pull/525" target="_blank" rel="noopener noreferrer">PR #525</a> 接口的宿主和匹配 RPC 版本的插件，其他标签页仍可使用
      </p>
      <ZButton :loading="state.busy" :disabled="dirty" @click="load">
        重新检查
      </ZButton>
    </section>
    <details class="section quota-settings quota-details">
      <summary>
        <span>查询设置</span>
        <svg class="quota-collapse" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
      </summary>
      <div class="quota-settings-body">
        <div class="quota-query-fields">
          <label class="field"><span>上游账号</span><ZSelect
            :model-value="state.accountId"
            :options="sourceOptions"
            :disabled="state.busy"
            filterable
            placeholder="选择上游账号"
            @update:model-value="state.accountId = $event ?? ''"
            @change="read"
          /></label>
          <label v-for="field in queryFields" :key="field.level" class="field"><span>{{ field.label }}</span><ZSelect
            :model-value="field.value"
            :options="field.options"
            :disabled="state.busy || !data || !support?.queryAvailable"
            filterable
            :placeholder="`选择${field.label.slice(2)}`"
            @update:model-value="selectQueryLevel(field.level, $event)"
          /></label>
        </div>
        <p v-if="data && state.queryType !== 'disabled'" class="muted">
          {{ resetHint }}
        </p>
        <p v-if="data && state.queryType !== 'disabled'" class="muted">
          使用上游账号的当前凭据和出站代理，保存后由插件在后台自动刷新
        </p>
        <div class="toolbar">
          <ZButton
            type="primary"
            :loading="state.busy"
            :disabled="!support?.queryAvailable || !dirty || (!source?.enabled && state.queryType !== 'disabled')"
            @click="save"
          >
            {{ state.queryType === 'disabled' ? '关闭查询' : '保存并获取' }}
          </ZButton>
        </div>
      </div>
    </details>
    <section class="section quota-results">
      <div class="toolbar quota-results-heading">
        <h2>额度详情</h2>
        <ZSelect
          v-if="!state.aggregate"
          :model-value="state.accountId"
          :options="sourceOptions"
          :disabled="state.busy || dirty"
          class="quota-result-source"
          aria-label="选择查看的上游账号"
          filterable
          placeholder="选择上游账号"
          @update:model-value="state.accountId = $event ?? ''"
          @change="read"
        />
        <div class="toolbar quota-results-actions">
          <ZSwitch v-model="state.aggregate" label="聚合" :disabled="state.busy || dirty" @change="syncCooldown" />
          <ZButton type="primary" :loading="state.busy" :disabled="!canRefresh" title="打开插件时刷新一次，之后每 3 分钟刷新" @click="refresh">
            刷新额度
          </ZButton>
        </div>
      </div>
      <p v-if="state.error" class="error" role="alert">
        {{ state.error }}
      </p>
      <p v-for="issue in issues" :key="issue.id" class="error" role="alert">
        {{ state.aggregate ? `${issue.name}：` : '' }}{{ issue.error }}
      </p>
      <p v-if="state.notice" class="muted" role="status">
        {{ state.notice }}
      </p>
      <p v-if="state.loaded && !state.sources.length" class="muted">
        暂无上游账号，请先在宿主添加 API 秘钥账号或通过「API 导入」接入
      </p>
      <p v-if="!state.aggregate && source && !source.enabled" class="muted">
        账号已停用，可查看已保存快照，恢复启用后再查询
      </p>
      <p v-for="blocked in blockedSources" :key="blocked.id" class="muted">
        {{ state.aggregate ? `${blocked.name}：` : '' }}上游认证或权限被拒绝，旧快照已停止下传，成功获取后恢复
      </p>
      <UpstreamQuotaCards
        v-if="cards.length"
        :cards="cards"
        :aggregate="state.aggregate"
        :active="active"
      />
      <p v-else-if="state.busy" class="muted" role="status">
        正在读取额度
      </p>
      <p v-else-if="!state.aggregate && data?.queryType === 'disabled'" class="muted">
        尚未开启额度查询，展开「查询设置」选择查询平台并保存
      </p>
      <p v-else-if="state.sources.length && !state.error" class="muted">
        {{ state.aggregate ? '暂无可展示的已配置上游额度' : data?.snapshot ? '上游未返回可展示的账号' : '尚无快照，可点击刷新额度获取' }}
      </p>
      <p v-if="!state.aggregate && data?.lastAttemptAt" class="muted quota-observed">
        最近查询尝试：{{ new Date(data.lastAttemptAt).toLocaleString() }}
      </p>
    </section>
    <QuotaSharing :active="active" :available="support?.sharingAvailable ?? false" />
  </div>
</template>
