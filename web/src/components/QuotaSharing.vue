<script setup lang="ts">
import type { Key } from '../api'
import type { QuotaSharing } from '../api/upstream'
import { ZButton, ZSelect, ZSwitch } from '@codex-proxy/ui'
import { computed, reactive, watch } from 'vue'
import { getKeys, message } from '../api'
import { getQuotaSharing, saveQuotaSharing } from '../api/upstream'

const props = defineProps<{ active: boolean, available: boolean }>()
const state = reactive({ keys: [] as Key[], keyId: '', value: null as QuotaSharing | null, saved: '', loaded: false, busy: false, error: '', notice: '' })
const dirty = computed(() => state.value !== null && JSON.stringify(state.value) !== state.saved)
function apply(value: QuotaSharing) {
  state.value = value
  state.saved = JSON.stringify(value)
}
async function read() {
  if (!props.available)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  state.value = null
  try {
    if (state.keyId)
      apply(await getQuotaSharing(state.keyId))
  }
  catch (error) { state.error = message(error) }
  finally { state.busy = false }
}
async function load() {
  if (!props.available)
    return
  state.busy = true
  state.error = ''
  try {
    state.keys = await getKeys()
    state.loaded = true
    if (!state.keys.some(key => key.id === state.keyId))
      state.keyId = state.keys[0]?.id ?? ''
  }
  catch (error) {
    state.error = message(error)
    state.busy = false
    return
  }
  await read()
}
async function save() {
  if (!state.value || !props.available)
    return
  state.busy = true
  state.error = ''
  state.notice = ''
  try {
    apply(await saveQuotaSharing(state.keyId, state.value))
    state.notice = state.value.enabled ? '已开启额度分享' : '已关闭额度分享'
  }
  catch (error) { state.error = message(error) }
  finally { state.busy = false }
}
watch(() => [props.active, props.available], ([active, available]) => {
  if (active && available && !state.loaded)
    void load()
}, { immediate: true })
</script>

<template>
  <section class="section">
    <h2>额度分享</h2>
    <div class="quota-sharing-controls">
      <ZSelect
        v-model="state.keyId" :options="state.keys.map((key) => ({ value: key.id, label: `${key.name}${key.enabled ? '' : '（已停用）'}` }))"
        :disabled="state.busy || !available" filterable placeholder="选择 Client Key" @change="read"
        class="quota-sharing-key" aria-label="选择分享额度的 Client Key"
      />
      <template v-if="state.value">
        <div class="quota-sharing-switches">
          <ZSwitch v-model="state.value.enabled" :disabled="state.busy || !available" label="开启分享" />
          <ZSwitch v-model="state.value.includeNames" :disabled="state.busy || !available || !state.value.enabled" label="账号名称" aria-label="分享账号名称" />
          <ZSwitch v-model="state.value.includeModels" :disabled="state.busy || !available || !state.value.enabled" label="模型列表" aria-label="分享模型列表" />
        </div>
        <div class="toolbar quota-sharing-actions">
          <ZButton type="primary" :disabled="!available || !dirty" :loading="state.busy" @click="save">
            保存设置
          </ZButton>
          <ZButton :disabled="state.busy || !available" @click="load">
            重新读取
          </ZButton>
        </div>
      </template>
    </div>
    <p class="muted">
      下游使用此 Client Key 查询 /v1/quota，仅返回其有权访问的启用账号，停用插件后不再提供透传
    </p>
    <p v-if="state.loaded && !state.keys.length" class="muted">
      暂无 Client Key，请先在宿主创建
    </p>
    <p v-if="state.error" class="error" role="alert">
      {{ state.error }}
    </p>
    <p v-if="state.notice" class="muted" role="status">
      {{ state.notice }}
    </p>
  </section>
</template>
