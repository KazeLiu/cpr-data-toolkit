<script setup lang="ts">
import { computed, onMounted, reactive } from 'vue'
import { ZButton, ZDialog, ZInput, ZSelect } from '@codex-proxy/ui'
import { getSummary, message } from '../api'
const state = reactive({
  period: 'today',
  summary: null as Awaited<ReturnType<typeof getSummary>> | null,
  tokens: '0',
  requests: '0',
  busy: false,
  error: '',
  notice: '',
  confirm: false,
})
function delta(value: string): bigint | null {
  return /^[+-]?\d{1,38}$/.test(value) ? BigInt(value) : null
}
const preview = computed(() => {
  const tokens = delta(state.tokens)
  const requests = delta(state.requests)
  if (tokens === null || requests === null)
    return { error: '请填写正负整数增减量', tokens: null, requests: null }
  if (!state.summary) return { error: '', tokens: null, requests: null }
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
function confirm() {
  state.confirm = false
  state.notice = '上游接口未开放'
}
onMounted(load)
</script>

<template>
  <section class="panel">
    <p class="notice">上游接口未开放 · 当前仅支持调整预览</p>
    <div class="toolbar">
      <ZSelect
        :model-value="state.period"
        @update:model-value="state.period = $event ?? ''"
        aria-label="统计范围"
        :options="[
          { value: 'today', label: '今天' },
          { value: '7d', label: '近 7 天' },
          { value: '30d', label: '近 30 天' },
        ]"
        :disabled="state.busy"
        style="width: 160px"
        @change="load"
      /><ZButton :loading="state.busy" @click="load">读取当前统计</ZButton
      ><span class="muted">以下总量属于所选范围，不代表全部历史</span>
    </div>
    <p v-if="state.error" class="error" role="alert">{{ state.error }}</p>
    <div class="form-grid">
      <label class="field"
        ><span>Token 总量增减</span
        ><ZInput
          v-model="state.tokens"
          maxlength="39"
          inputmode="text"
          placeholder="例如 1000 或 -1000"
      /></label>
      <label class="field"
        ><span>请求次数增减</span
        ><ZInput
          v-model="state.requests"
          maxlength="39"
          inputmode="text"
          placeholder="例如 10 或 -10"
      /></label>
    </div>
    <div class="section">
      <div class="metrics">
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
    </div>
    <p v-if="preview.error" class="error" role="alert">{{ preview.error }}</p>
    <p v-if="state.notice" class="notice" role="status">{{ state.notice }}</p>
    <div class="toolbar">
      <ZButton
        type="primary"
        :disabled="
          state.busy ||
          !!preview.error ||
          (delta(state.tokens) === 0n && delta(state.requests) === 0n)
        "
        @click="state.confirm = true"
        >预览调整</ZButton
      >
    </div>
    <ZDialog v-model="state.confirm" title="确认 Token 与请求次数增减" type="warning"
      ><div class="panel">
        <dl>
          <dt>Token 增减</dt>
          <dd class="mono">{{ state.tokens }}</dd>
          <dt>请求次数增减</dt>
          <dd class="mono">{{ state.requests }}</dd>
        </dl>
        <p>
          未提供模型、费用和耗时信息，总计费、首字均值等指标无法随补录量完整更新，可能与调整后的总量不一致
        </p>
        <p class="muted">当前版本仅预览，确认后不会修改统计</p>
      </div>
      <template #footer
        ><ZButton @click="state.confirm = false">取消</ZButton
        ><ZButton type="primary" @click="confirm">确认调整</ZButton></template
      ></ZDialog
    >
  </section>
</template>
