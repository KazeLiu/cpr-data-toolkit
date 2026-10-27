<script setup lang="ts">
import type { QuotaAmount, QuotaSource, QuotaWindow, UpstreamAccount } from '../api/upstream'
import { ZButton, ZDialog, ZScrollbar, ZTag } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import QuotaModels from './QuotaModels.vue'

const props = defineProps<{
  cards: { source: QuotaSource, account: UpstreamAccount, stale: boolean }[]
  aggregate: boolean
  active: boolean
}>()
const models = reactive({ open: false, sourceId: '', accountId: '' })
const displayCards = computed(() => props.cards.map(card => ({
  ...card,
  amounts: card.account.amounts.filter(entry => !isCredits(entry)
    || entry.unlimited
    || entry.remaining !== null
    || entry.used !== null
    || entry.total !== null
    || entry.details.some(detail => detail.amount !== null)
    || entry.invalidMessage !== null),
})))
const selectedCard = computed(() => props.cards.find(card => card.source.id === models.sourceId && card.account.id === models.accountId))
watch([() => props.active, () => selectedCard.value?.account.models], ([active, items]) => {
  if (!active || !items?.length)
    models.open = false
})

function showModels(sourceId: string, accountId: string) {
  models.sourceId = sourceId
  models.accountId = accountId
  models.open = true
}

function isMonthly(window: QuotaWindow) {
  return window.role === 'monthly' || (window.windowSeconds !== null && window.windowSeconds >= 28 * 86400)
}
function isCredits(entry: QuotaAmount) {
  return entry.label === '点数余额' || entry.label === '额外点数余额'
}
function sortedWindows(windows: QuotaWindow[]) {
  return [...windows].sort((left, right) => Number(isMonthly(left)) - Number(isMonthly(right)))
}

function date(value: string | null) {
  return value === null ? '未提供' : new Date(value).toLocaleString()
}
function percentText(used: number | null) {
  return used === null
    ? '未知'
    : `已用 ${used.toLocaleString(undefined, { maximumFractionDigits: 1 })}%`
}
function tone(used: number | null) {
  return used === null ? 'unknown' : used >= 95 ? 'danger' : used >= 80 ? 'warning' : 'normal'
}
</script>

<template>
  <div class="quota-cards">
    <section v-for="{ source, account, stale, amounts } in displayCards" :key="`${source.id}:${account.id}`" class="quota-card">
      <ZScrollbar max-height="22rem" class="quota-card-scroll">
        <header class="quota-card-heading">
          <div class="quota-account-title">
            <div class="quota-account-name">
              <h3>{{ account.label }}</h3>
              <span v-if="stale || account.status === 'stale'" class="quota-warning">数据过期</span>
            </div>
            <ZTag v-if="aggregate" size="small" class="quota-source-tag" :title="source.name">{{ source.name }}</ZTag>
          </div>
          <div class="quota-card-meta">
            <span class="muted">最后获取：{{ date(account.observedAt) }}</span>
            <span v-if="account.models === null" class="muted">模型未提供</span>
            <ZButton v-else type="primary" variant="text" size="small" class="quota-model-button" :disabled="!account.models.length" aria-haspopup="dialog" @click="showModels(source.id, account.id)">
              {{ account.models.length }} 个模型
            </ZButton>
          </div>
        </header>
        <div class="quota-card-content">
          <p v-if="account.message" class="quota-warning" role="status">
            {{ account.message }}
          </p>
          <div
            v-if="account.windows.length"
            class="quota-windows"
          >
            <div
              v-for="window in sortedWindows(account.windows)"
              :key="window.key"
              class="quota-window"
              :class="{ 'quota-window-monthly': isMonthly(window) }"
              :data-tone="tone(window.usedPercent)"
            >
              <div class="quota-window-heading">
                <span>{{ window.label }}</span><strong class="mono">{{ percentText(window.usedPercent) }}</strong>
              </div>
              <div
                class="quota-track"
                role="progressbar"
                :aria-label="window.label"
                :aria-valuenow="window.usedPercent ?? undefined"
                :aria-valuetext="percentText(window.usedPercent)"
                :aria-valuemin="0"
                :aria-valuemax="100"
              >
                <div class="quota-fill" :style="{ width: `${window.usedPercent ?? 0}%` }" />
              </div>
              <div class="quota-reset muted">
                <span>重置</span><time :datetime="window.resetAt ?? undefined">{{ date(window.resetAt) }}</time>
              </div>
            </div>
          </div>
          <div v-if="amounts.length" class="quota-amounts">
            <div v-for="(entry, index) in amounts" :key="index" class="quota-amount">
              <span v-if="!entry.isValid" class="quota-warning">{{ entry.invalidMessage ?? (isCredits(entry) ? '额外点数不可用' : '额度不可用') }}</span>
              <div class="quota-amount-fields" :class="{ 'quota-balance-row': entry.details.length === 2 }">
                <div class="quota-balance">
                  <span class="muted">{{ isCredits(entry) ? '额外点数余额' : entry.label }}</span>
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
                  >{{ isCredits(entry) ? '余额未提供' : '额度未知' }}</span>
                </div>
                <div v-for="detail in entry.details" :key="detail.label" class="quota-balance-detail muted">{{ detail.label }} <span class="mono">{{ detail.amount ?? '未知' }} {{ entry.unit }}</span></div>
              </div>
              <span v-if="entry.resetAt" class="muted">重置 {{ date(entry.resetAt) }}</span>
            </div>
          </div>
          <span v-if="!account.windows.length && !amounts.length" class="muted">额度待观测</span>
        </div>
      </ZScrollbar>
    </section>
  </div>
  <ZDialog v-model="models.open" :title="`${selectedCard?.account.label ?? ''} · 模型详情`">
    <QuotaModels v-if="selectedCard?.account.models" :key="`${models.sourceId}:${models.accountId}`" :models="selectedCard.account.models" />
  </ZDialog>
</template>
