<script setup lang="ts">
import { ZButton } from '@codex-proxy/ui'

defineProps<{ offset: number, total: number, pageSize: number, busy?: boolean }>()
defineEmits<{ change: [offset: number] }>()
</script>

<template>
  <nav class="toolbar table-pagination" aria-label="表格分页">
    <ZButton :disabled="busy || offset === 0" @click="$emit('change', Math.max(0, offset - pageSize))">上一页</ZButton>
    <span class="muted">{{ total ? offset + 1 : 0 }}–{{ Math.min(offset + pageSize, total) }} / {{ total }}</span>
    <ZButton :disabled="busy || offset + pageSize >= total" @click="$emit('change', offset + pageSize)">下一页</ZButton>
  </nav>
</template>
