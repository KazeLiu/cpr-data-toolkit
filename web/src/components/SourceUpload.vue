<script setup lang="ts">
import { reactive, ref } from 'vue'
import { ZButton } from '@codex-proxy/ui'
import { integer, json, message, text } from '../api'

const props = defineProps<{
  sourceId: string
  filename: string
  locked: boolean
}>()
const emit = defineEmits<{
  changed: [id: string, filename: string]
  busy: [value: boolean]
}>()
const input = ref<HTMLInputElement>()
const upload = reactive({
  busy: false,
  clearing: false,
  cancelled: false,
  progress: 0,
  error: '',
  note: '',
})
function encode(bytes: Uint8Array): string {
  let text = ''
  for (const byte of bytes) text += String.fromCharCode(byte)
  return btoa(text)
}
async function choose(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (!file) return
  if (
    !file.name.toLowerCase().endsWith('.sql') ||
    file.size === 0 ||
    file.size > 64 * 1024 * 1024
  ) {
    upload.error = '请选择不超过 64 MiB 的 CCS 完整 SQL 导出'
    return
  }
  upload.busy = true
  upload.cancelled = false
  upload.progress = 0
  upload.error = ''
  upload.note = ''
  emit('busy', true)
  let sourceId = ''
  try {
    sourceId = text((await json('POST', 'api/uploads/start', { size: file.size })).sourceId)
    for (let offset = 0; offset < file.size; offset += 256 * 1024) {
      if (upload.cancelled) throw new Error('上传已取消')
      const chunk = new Uint8Array(await file.slice(offset, offset + 256 * 1024).arrayBuffer())
      const result = await json('POST', 'api/uploads/chunk', {
        sourceId,
        offset,
        data: encode(chunk),
      })
      if (integer(result.offset) !== offset + chunk.length)
        throw new Error('上传偏移不一致，请重新上传')
      upload.progress = Math.round(((offset + chunk.length) * 100) / file.size)
    }
    if (upload.cancelled) throw new Error('上传已取消')
    await json('POST', 'api/uploads/finish', { sourceId })
    if (upload.cancelled) throw new Error('上传已取消')
    const previous = props.sourceId
    emit('changed', sourceId, file.name)
    if (previous) {
      try {
        await json('POST', 'api/uploads/cancel', { sourceId: previous })
      } catch (error) {
        upload.note = `上一份来源清理未确认，到期自动清理：${message(error)}`
      }
    }
    sourceId = ''
  } catch (error) {
    upload.error = message(error)
    if (sourceId) {
      try {
        await json('POST', 'api/uploads/cancel', { sourceId })
      } catch (cleanup) {
        upload.error += `；清理未确认，到期自动清理：${message(cleanup)}`
      }
    }
  } finally {
    upload.busy = false
    emit('busy', false)
    if (input.value) input.value.value = ''
  }
}
async function clear() {
  upload.error = ''
  upload.clearing = true
  emit('busy', true)
  try {
    await json('POST', 'api/uploads/cancel', { sourceId: props.sourceId })
    emit('changed', '', '')
  } catch (error) {
    upload.error = message(error)
  } finally {
    upload.clearing = false
    emit('busy', false)
  }
}
</script>

<template>
  <section class="section" style="margin-bottom: 16px">
    <div class="toolbar">
      <input
        ref="input"
        type="file"
        accept=".sql"
        hidden
        :disabled="locked || upload.busy || upload.clearing"
        @change="choose"
      />
      <ZButton :disabled="locked || upload.busy || upload.clearing" @click="input?.click()"
        >选择 CCS 完整 SQL 导出</ZButton
      >
      <ZButton v-if="upload.busy" variant="text" @click="upload.cancelled = true">取消上传</ZButton>
      <ZButton
        v-else-if="sourceId"
        variant="text"
        :disabled="locked"
        :loading="upload.clearing"
        @click="clear"
        >清理来源</ZButton
      >
      <span v-if="upload.busy" aria-live="polite"
        >{{ upload.progress }}% ·
        {{ upload.progress === 100 ? '正在检查来源结构' : '上传中' }}</span
      >
      <span v-else-if="filename" class="mono">{{ filename }}</span>
    </div>
    <p class="muted">
      使用 CCS 手动完整导出，不使用云同步导出 · 最多 64 MiB · 来源仅在后端内存暂存，闲置 15
      分钟后清理
    </p>
    <p v-if="upload.error" class="error" role="alert">{{ upload.error }}</p>
    <p v-if="upload.note" class="notice">{{ upload.note }}</p>
  </section>
</template>
