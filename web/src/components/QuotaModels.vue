<script setup lang="ts">
import { ZInput, ZNotification, ZScrollbar, ZTag } from '@codex-proxy/ui'
import { computed, ref } from 'vue'
import { copyText } from '../clipboard'

const props = defineProps<{ models: string[] }>()
const search = ref('')
const filtered = computed(() => {
  const keyword = search.value.trim().toLocaleLowerCase()
  return props.models.filter(model => model.toLocaleLowerCase().includes(keyword))
})

async function copy(model: string) {
  try {
    await copyText(model)
    ZNotification.success({ message: '已复制', duration: 1500, deduplicate: true })
  }
  catch {
    ZNotification.error({ message: '复制失败，请检查浏览器剪贴板权限', deduplicate: true })
  }
}
</script>

<template>
  <div class="quota-model-content">
    <ZInput v-model="search" placeholder="搜索模型" aria-label="搜索此账号的模型" clearable />
    <span class="muted">{{ filtered.length }} / {{ models.length }} 个模型</span>
    <ZScrollbar v-if="filtered.length" max-height="min(24rem, 50dvh)" always-visible>
      <div class="quota-model-tags">
        <button v-for="model in filtered" :key="model" type="button" class="quota-model-copy" :aria-label="`复制模型 ${model}`" :title="`复制 ${model}`" @click="copy(model)">
          <ZTag class="mono">
            {{ model }}
          </ZTag>
        </button>
      </div>
    </ZScrollbar>
    <span v-else class="muted">{{ models.length ? '没有匹配的模型' : '暂无可见模型' }}</span>
  </div>
</template>
