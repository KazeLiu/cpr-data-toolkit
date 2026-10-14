<script setup lang="ts">
import { ZTable } from '@codex-proxy/ui'
import type { TableColumn } from '@codex-proxy/ui'
import type { OperationResult } from '../api'
defineProps<{ results: OperationResult[] }>()
const labels: Record<string, string> = {
  saved: '已保存',
  skipped: '已跳过',
  failed: '失败',
  unknown: '结果未知',
  mapping_pending: '映射待恢复',
  not_run: '未提交',
}
const columns: TableColumn<OperationResult>[] = [
  { key: 'label', label: '对象', kind: 'mono', size: 'lg' },
  {
    key: 'status',
    label: '结果',
    kind: 'status',
    size: 'sm',
    formatter: (value) => labels[String(value)] ?? String(value),
  },
  { key: 'message', label: '说明', size: '3xl' },
]
</script>
<template>
  <ZTable v-if="results.length" :columns="columns" :data="results" size="small" />
</template>
