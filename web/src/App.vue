<script setup lang="ts">
import { ZButton } from '@codex-proxy/ui'
import { computed, onBeforeUnmount, onMounted, reactive } from 'vue'
import { host, message } from './api'
import SourceUpload from './components/SourceUpload.vue'
import ApiImport from './features/ApiImport.vue'
import ManualAdjust from './features/ManualAdjust.vue'
import ModelUa from './features/ModelUa.vue'
import Pricing from './features/Pricing.vue'
import UsageImport from './features/UsageImport.vue'

const tabs = ['API 导入', '用量导入', '手动调整', '模型定价', '模型UA定制']
const page = reactive({
  tab: 0,
  ready: false,
  error: '',
  sourceId: '',
  filename: '',
  uploadBusy: false,
  apiImportBusy: false,
  usageImportBusy: false,
})
const importBusy = computed(() => page.apiImportBusy || page.usageImportBusy)
function theme() {
  document.documentElement.dataset.theme = host().theme
}
function navigate(event: KeyboardEvent) {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
  event.preventDefault()
  page.tab =
    event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? tabs.length - 1
        : (page.tab + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length
  document.getElementById(`tab-${page.tab}`)?.focus()
}
onMounted(() => {
  try {
    theme()
    page.ready = true
    window.addEventListener('codex-proxy-themechange', theme)
  } catch (error) {
    page.error = message(error)
  }
})
onBeforeUnmount(() => window.removeEventListener('codex-proxy-themechange', theme))
</script>

<template>
  <main class="toolkit">
    <p v-if="page.error" class="error" role="alert">{{ page.error }}</p>
    <template v-if="page.ready">
      <nav class="tabs" role="tablist" aria-label="工具箱功能" @keydown="navigate">
        <ZButton
          v-for="(tab, index) in tabs"
          :id="`tab-${index}`"
          :key="tab"
          role="tab"
          :aria-selected="page.tab === index"
          :aria-controls="`panel-${index}`"
          :tabindex="page.tab === index ? 0 : -1"
          :type="page.tab === index ? 'primary' : 'default'"
          :variant="page.tab === index ? 'solid' : 'text'"
          @click="page.tab = index"
        >
          {{ tab }}
        </ZButton>
      </nav>
      <SourceUpload
        v-show="page.tab < 2"
        :source-id="page.sourceId"
        :filename="page.filename"
        :locked="importBusy"
        @busy="page.uploadBusy = $event"
        @changed="
          (id, filename) => {
            page.sourceId = id
            page.filename = filename
          }
        "
      />
      <div v-show="page.tab === 0" id="panel-0" role="tabpanel" aria-labelledby="tab-0">
        <ApiImport
          :source-id="page.sourceId"
          :locked="page.uploadBusy || page.usageImportBusy"
          :active="page.tab === 0"
          @busy="page.apiImportBusy = $event"
        />
      </div>
      <div v-show="page.tab === 1" id="panel-1" role="tabpanel" aria-labelledby="tab-1">
        <UsageImport
          :source-id="page.sourceId"
          :locked="page.uploadBusy || page.apiImportBusy"
          @busy="page.usageImportBusy = $event"
        />
      </div>
      <div v-show="page.tab === 2" id="panel-2" role="tabpanel" aria-labelledby="tab-2">
        <ManualAdjust />
      </div>
      <div v-show="page.tab === 3" id="panel-3" role="tabpanel" aria-labelledby="tab-3">
        <Pricing />
      </div>
      <div v-show="page.tab === 4" id="panel-4" role="tabpanel" aria-labelledby="tab-4">
        <ModelUa />
      </div>
    </template>
  </main>
</template>
