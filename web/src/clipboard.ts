export async function copyText(value: string) {
  const policy = (document as Document & { featurePolicy?: { allowsFeature: (feature: string) => boolean } }).featurePolicy
  // 隔离页面可能禁用现代剪贴板接口，此时同步复制才能保留本次点击的用户手势
  if (navigator.clipboard?.writeText && policy?.allowsFeature('clipboard-write') !== false) {
    await navigator.clipboard.writeText(value)
    return
  }
  const input = document.createElement('textarea')
  input.value = value
  input.style.position = 'fixed'
  input.style.opacity = '0'
  document.body.append(input)
  const focused = document.activeElement as HTMLElement | null
  try {
    input.select()
    if (!document.execCommand('copy'))
      throw new Error('复制失败，请检查浏览器剪贴板权限')
  }
  finally {
    input.remove()
    focused?.focus()
  }
}
