<script setup lang="ts">
import type { UpstreamAccount } from '../api/upstream'
import { ZButton, ZDialog, ZScrollbar } from '@codex-proxy/ui'
import { reactive, watch } from 'vue'

const props = defineProps<{
  accounts: UpstreamAccount[]
  stale: boolean
  remaining: boolean
  active: boolean
}>()
const models = reactive({ open: false, account: null as UpstreamAccount | null })
watch([() => props.accounts, () => props.active], () => {
  models.open = false
})

function date(value: string | null) {
  return value === null ? '未提供' : new Date(value).toLocaleString()
}
function percent(used: number | null) {
  return used === null ? null : props.remaining ? 100 - used : used
}
function percentText(used: number | null) {
  const value = percent(used)
  return value === null
    ? '未知'
    : `${props.remaining ? '剩余' : '已用'} ${value.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`
}
function tone(used: number | null) {
  return used === null ? 'unknown' : used >= 95 ? 'danger' : used >= 80 ? 'warning' : 'normal'
}
</script>

<template>
  <div class="quota-cards">
    <section v-for="account in accounts" :key="account.id" class="quota-card">
      <header class="quota-card-heading">
        <div class="quota-account-title">
          <h3>{{ account.label }}</h3>
          <ZButton
            v-if="account.models?.length"
            type="primary"
            variant="text"
            size="small"
            aria-haspopup="dialog"
            @click="
              models.account = account;
              models.open = true
            "
          >
            模型详情
          </ZButton>
          <span v-if="stale || account.status === 'stale'" class="quota-warning">数据过期</span>
        </div>
        <span v-if="account.observedAt" class="muted quota-observed">上游额度最后获取时间：{{ date(account.observedAt) }}</span>
      </header>
      <p v-if="account.message" class="quota-warning" role="status">
        {{ account.message }}
      </p>
      <div
        v-if="account.windows.length"
        class="quota-windows"
        :class="{ 'quota-single': account.windows.length === 1 }"
      >
        <div
          v-for="window in account.windows"
          :key="window.key"
          class="quota-window"
          :data-tone="tone(window.usedPercent)"
        >
          <div class="quota-window-heading">
            <span>{{ window.label }}</span><strong class="mono">{{ percentText(window.usedPercent) }}</strong>
          </div>
          <div
            class="quota-track"
            role="progressbar"
            :aria-label="window.label"
            :aria-valuenow="percent(window.usedPercent) ?? undefined"
            :aria-valuetext="percentText(window.usedPercent)"
            :aria-valuemin="0"
            :aria-valuemax="100"
          >
            <div class="quota-fill" :style="{ width: `${percent(window.usedPercent) ?? 0}%` }" />
          </div>
          <div class="quota-reset muted">
            <span>重置</span><time :datetime="window.resetAt ?? undefined">{{ date(window.resetAt) }}</time>
          </div>
        </div>
      </div>
      <div v-if="account.amounts.length" class="quota-amounts">
        <div v-for="(entry, index) in account.amounts" :key="index" class="quota-amount">
          <span class="muted">{{ entry.label }}</span>
          <span v-if="!entry.isValid" class="quota-warning">{{ entry.invalidMessage ?? '额度不可用' }}</span>
          <strong v-if="entry.unlimited">不限额</strong>
          <strong v-else-if="entry.remaining !== null" class="mono">{{ entry.remaining }} {{ entry.unit }}</strong>
          <span v-if="entry.used !== null || entry.total !== null" class="mono">已用 {{ entry.used ?? '未知' }} / {{ entry.total ?? '未知' }} {{ entry.unit }}</span>
          <span
            v-if="
              !entry.unlimited
                && entry.remaining === null
                && entry.used === null
                && entry.total === null
            "
            class="muted"
          >额度未知</span>
          <span v-if="entry.resetAt" class="muted">重置 {{ date(entry.resetAt) }}</span>
          <span v-for="detail in entry.details" :key="detail.label" class="muted">{{ detail.label }} <span class="mono">{{ detail.amount ?? '未知' }} {{ entry.unit }}</span></span>
        </div>
      </div>
      <span v-if="!account.windows.length && !account.amounts.length" class="muted">额度待观测</span>
    </section>
    <ZDialog
      v-model="models.open"
      title="账号模型"
      :description="models.account?.label"
      width="36rem"
    >
      <ZScrollbar max-height="24rem">
        <ul class="quota-models">
          <li v-for="model in models.account?.models" :key="model" class="mono">
            {{ model }}
          </li>
        </ul>
      </ZScrollbar>
      <template #footer>
        <ZButton @click="models.open = false">
          关闭
        </ZButton>
      </template>
    </ZDialog>
  </div>
</template>
