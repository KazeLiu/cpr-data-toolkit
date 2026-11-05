<script setup lang="ts">
import type { Adjustment } from '../api'
import { ZButton, ZDialog, ZInput, ZSelect } from '@codex-proxy/ui'
import { computed, onMounted, reactive } from 'vue'
import { ApiError, getPendingAdjustment, getSummary, message, saveAdjustment } from '../api'

const state = reactive({
  period: 'today',
  summary: null as Awaited<ReturnType<typeof getSummary>> | null,
  tokens: '0',
  requests: '0',
  busy: false,
  error: '',
  notice: '',
  confirm: false,
  ready: false,
  pending: null as Adjustment | null,
})
function delta(value: string): bigint | null {
  if (!/^[+-]?\d{1,19}$/.test(value)) return null
  const number = BigInt(value)
  return number >= -9223372036854775808n && number <= 9223372036854775807n ? number : null
}
const preview = computed(() => {
  const tokens = delta(state.tokens)
  const requests = delta(state.requests)
  if (tokens === null || requests === null)
    return { error: '请填写宿主 64 位整数范围内的增减量', tokens: null, requests: null }
  if (!state.summary) return { error: '请先读取当前统计', tokens: null, requests: null }
  const nextTokens = BigInt(state.summary.tokens) + tokens
  const nextRequests = BigInt(state.summary.requests) + requests
  return {
    error: nextTokens < 0n || nextRequests < 0n ? '预览总量不能为负数' : '',
    tokens: nextTokens.toString(),
    requests: nextRequests.toString(),
  }
})
async function load() {
  state.busy = true
  state.error = ''
  state.summary = null
  try {
    state.summary = await getSummary(state.period)
  } catch (error) {
    state.error = `当前统计不可用：${message(error)}`
  } finally {
    state.busy = false
  }
}
async function initialize() {
  state.busy = true
  try {
    state.pending = await getPendingAdjustment()
    if (state.pending) {
      state.tokens = state.pending.tokens
      state.requests = state.pending.requests
      state.notice = '发现结果未确认的调整，请继续核对原操作，不要另建修正'
    }
    state.ready = true
  } catch (error) {
    state.error = message(error)
  } finally {
    state.busy = false
  }
  if (state.ready) await load()
}
async function confirm() {
  state.confirm = false
  state.pending ??= {
    id: crypto.randomUUID(),
    atMs: Date.now(),
    tokens: BigInt(state.tokens).toString(),
    requests: BigInt(state.requests).toString(),
  }
  state.busy = true
  state.error = ''
  try {
    await saveAdjustment(state.pending)
    state.notice = '调整已保存'
    state.pending = null
    state.tokens = '0'
    state.requests = '0'
    await load()
  } catch (error) {
    state.error = `${message(error)}；已保留本次调整，点击继续核对调整，不要重复创建`
    if (error instanceof ApiError && error.status < 500) {
      try {
        state.pending = await getPendingAdjustment()
        if (!state.pending) state.error = message(error)
      } catch (readError) {
        state.error += `；读取操作状态失败：${message(readError)}`
      }
    }
  } finally {
    state.busy = false
  }
}
onMounted(initialize)
</script>

<template>
  <section class="section">
    <h2>调整 Token 与请求数</h2>
    <div class="filter-bar">
      <label class="field adjust-range"><span>统计范围</span>
      <ZSelect
        :model-value="state.period"
        aria-label="统计范围"
        :options="[
          { value: 'today', label: '今天' },
          { value: '7d', label: '近 7 天' },
          { value: '30d', label: '近 30 天' },
        ]"
        :disabled="state.busy"
        @update:model-value="state.period = $event ?? ''"
        @change="load"
      /></label><ZButton :loading="state.busy" @click="state.ready ? load() : initialize()">
        读取当前统计 </ZButton
      >
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <div class="form-grid">
      <label class="field"
        ><span>Token 总量增减</span
        ><ZInput
          v-model="state.tokens"
          maxlength="20"
          :disabled="state.busy || !!state.pending"
          inputmode="text"
          placeholder="例如 1000 或 -1000"
      /></label>
      <label class="field"
        ><span>请求次数增减</span
        ><ZInput
          v-model="state.requests"
          maxlength="20"
          :disabled="state.busy || !!state.pending"
          inputmode="text"
          placeholder="例如 10 或 -10"
      /></label>
    </div>
    <div class="metrics adjust-preview">
      <div>
        当前 Token<strong>{{ state.summary?.tokens ?? '当前统计不可用' }}</strong>
      </div>
      <div>
        调整后预览<strong>{{ preview.tokens ?? '—' }}</strong>
      </div>
      <div>
        当前请求数<strong>{{ state.summary?.requests ?? '当前统计不可用' }}</strong>
      </div>
      <div>
        调整后预览<strong>{{ preview.requests ?? '—' }}</strong>
      </div>
    </div>
    <p v-if="preview.error" class="error" role="alert">{{ preview.error }}</p>
    <p v-if="state.notice" :class="state.pending ? 'notice' : 'muted'" role="status">{{ state.notice }}</p>
    <div class="toolbar">
      <ZButton v-if="state.pending" type="primary" :loading="state.busy" @click="confirm">
        继续核对调整
      </ZButton>
      <ZButton
        v-else
        type="primary"
        :disabled="
          state.busy ||
          !state.ready ||
          !!preview.error ||
          (delta(state.tokens) === 0n && delta(state.requests) === 0n)
        "
        @click="state.confirm = true"
      >
        预览调整
      </ZButton>
    </div>
    <ZDialog v-model="state.confirm" title="确认 Token 与请求次数增减" type="warning"
      ><div class="panel">
        <dl>
          <dt>Token 增减</dt>
          <dd class="mono">{{ state.tokens }}</dd>
          <dt>请求次数增减</dt>
          <dd class="mono">{{ state.requests }}</dd>
        </dl>
        <p>仅调整 Token 与请求数，费用和预算不变</p>
        <p class="notice">减量不能超过首次提交时刻所在 15 分钟桶的已有用量</p>
      </div>
      <template #footer
        ><ZButton @click="state.confirm = false">取消</ZButton
        ><ZButton type="primary" @click="confirm">确认调整</ZButton></template
      ></ZDialog
    >
  </section>
</template>

<style scoped>
.filter-bar > .adjust-range {
  flex: 0 1 12rem;
  max-width: 100%;
}
.adjust-preview {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}
@container toolkit (max-width: 800px) {
  .adjust-preview {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
