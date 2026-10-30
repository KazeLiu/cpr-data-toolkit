export function localDateTime(date = new Date()): string {
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000).toISOString().slice(0, 16)
}

export function formatDateTime(value: string | number): string {
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}
