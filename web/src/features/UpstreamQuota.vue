<script setup lang="ts">
import type { QueryType, QuotaSource, UpstreamState } from '../api/upstream'
import { ZButton, ZScrollbar, ZSelect } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { message } from '../api'
import {
  getQuotaSources,
  getUpstream,
  queryOptions,
  refreshUpstream,
  saveUpstream,
} from '../api/upstream'
import QuotaSharing from '../components/QuotaSharing.vue'
import UpstreamQuotaCards from '../components/UpstreamQuotaCards.vue'

const props = defineProps<{ active: boolean }>()
const state = reactive({
  sources: [] as QuotaSource[],
  accountId: '',
  queryType: 'disabled' as QueryType,
  data: null as UpstreamState | null,
  loaded: false,
  busy: false,
  error: '',
  notice: '',
})
const dirty = computed(() => state.data !== null && state.queryType !== state.data.queryType)
const source = computed(() => state.sources.find(source => source.id === state.accountId))
const canRefresh = computed(
  () =>
    state.data !== null
    && state.data.queryType !== 'disabled'
    && !dirty.value
    && !state.busy
    && source.value?.enabled,
)
const resetHint = computed(() =>
  state.queryType === 'cpr'
    ? '上游 CPR 需为此 Key 开启额度共享，名称和模型按上游分享设置显示'
    : state.queryType.startsWith('kimi')
      ? 'Kimi 查询使用所选地区的官方额度接口'
      : '使用账号 API 地址和当前 Key 查询，不自动探测上游类型',
)

function apply(data: UpstreamState) {
  state.data = data
  state.queryType = data.queryType
}

async function read() {
  if (!state.accountId) {
    state.data = null
    return
  }
  state.busy = true
  state.error = ''
  state.notice = ''
  state.data = null
  try {
    apply(await getUpstream(state.accountId))
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}

async function loadSources() {
  state.busy = true
  state.error = ''
  state.notice = ''
  state.data = null
  try {
    state.sources = await getQuotaSources()
    state.loaded = true
    if (!state.sources.some(source => source.id === state.accountId)) {
      state.accountId
        = state.sources.find(source => source.enabled)?.id ?? state.sources[0]?.id ?? ''
    }
    if (state.accountId)
      apply(await getUpstream(state.accountId))
    else state.data = null
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}

async function save() {
  if (!state.data)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    apply(await saveUpstream(state.accountId, state.data.version, state.queryType))
    state.notice = state.data.error
      ? '设置已保存，本次获取失败'
      : state.queryType === 'disabled'
        ? '已关闭查询'
        : '设置已保存，宿主将自动获取并定期刷新额度'
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}

async function refresh() {
  if (!canRefresh.value)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    apply(await refreshUpstream(state.accountId))
  }
  catch (error) {
    state.error = message(error)
  }
  finally {
    state.busy = false
  }
}

watch(
  () => props.active,
  (active) => {
    if (active && !state.loaded)
      void loadSources()
  },
  { immediate: true },
)
</script>

<template>
  <div class="panel">
    <div class="section">
      <div class="form-grid">
        <label class="field"><span>宿主账号</span><ZSelect
          :model-value="state.accountId"
          :options="
            state.sources.map((source) => ({
              value: source.id,
              label: `${source.name}${source.enabled ? '' : '（已停用）'}`,
            }))
          "
          :disabled="state.busy"
          filterable
          placeholder="选择 API Key 账号"
          @update:model-value="state.accountId = $event ?? ''"
          @change="read"
        /></label>
        <label class="field"><span>查询类型</span><ZSelect
          :model-value="state.queryType"
          :options="queryOptions"
          :disabled="state.busy || !state.data"
          @update:model-value="state.queryType = ($event ?? 'disabled') as QueryType"
        /></label>
      </div>
      <p v-if="state.data && state.queryType !== 'disabled'" class="muted">
        {{ resetHint }}
      </p>
      <p v-if="state.data && state.queryType !== 'disabled'" class="muted">
        宿主使用账号当前凭据和出站代理查询，后台自动刷新
      </p>
      <div class="toolbar">
        <ZButton
          type="primary"
          :loading="state.busy"
          :disabled="!dirty || (!source?.enabled && state.queryType !== 'disabled')"
          @click="save"
        >
          {{ state.queryType === 'disabled' ? '关闭查询' : '保存并获取' }}
        </ZButton>
        <ZButton
          v-if="state.data?.queryType !== 'disabled' && state.data"
          :disabled="!canRefresh"
          @click="refresh"
        >
          刷新额度
        </ZButton>
        <ZButton variant="text" :disabled="state.busy" @click="loadSources">
          重新读取
        </ZButton>
      </div>
      <p v-if="state.error" class="error" role="alert">
        {{ state.error }}
      </p>
      <p v-if="state.notice" class="muted" role="status">
        {{ state.notice }}
      </p>
      <p v-if="state.loaded && !state.sources.length" class="muted">
        暂无 API Key 账号，请先在宿主添加账号或通过「API 导入」接入
      </p>
      <p v-if="source && !source.enabled" class="muted">
        账号已停用，可查看已保存快照，恢复启用后再查询
      </p>
    </div>
    <div v-if="state.data && state.data.queryType !== 'disabled'" class="section">
      <p v-if="state.data.error" class="error" role="alert">
        {{ state.data.error }}
      </p>
      <p v-if="state.data.forwardingBlocked" class="muted">
        上游认证或权限被拒绝，旧快照已停止下传，成功获取后恢复
      </p>
      <p v-if="state.data.lastAttemptAt" class="muted">
        最近查询尝试：{{ new Date(state.data.lastAttemptAt).toLocaleString() }}
      </p>
      <details
        v-if="state.data.snapshot?.accounts.length"
        :key="`${state.accountId}:${state.data.queryType}`"
        open
        class="quota-details"
      >
        <summary>
          <span>{{
            state.data.queryType === 'cpr'
              ? `上游账号 ${state.data.snapshot.accounts.length} 个`
              : '上游额度'
          }}</span><svg
            class="quota-collapse"
            width="16"
            height="16"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            aria-hidden="true"
          >
            <path d="m6 9 6 6 6-6" />
          </svg>
        </summary>
        <ZScrollbar max-height="28rem">
          <UpstreamQuotaCards
            :accounts="state.data.snapshot.accounts"
            :active="active"
            :stale="state.data.stale"
            :remaining="state.data.queryType === 'cpr'"
          />
        </ZScrollbar>
      </details>
      <p v-else class="muted">
        {{ state.data.snapshot ? '上游未返回可展示的账号' : '尚无快照，宿主后台获取后可重新读取' }}
      </p>
    </div>
    <QuotaSharing :active="active" />
  </div>
</template>
