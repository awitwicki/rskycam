export function formatExposure(us: number): string {
  const s = us / 1e6
  if (s >= 1) return `${s % 1 === 0 ? s : s.toFixed(1)} s`
  let trimmed = s.toFixed(6)
  while (trimmed.endsWith('0')) trimmed = trimmed.slice(0, -1)
  if (trimmed.endsWith('.')) trimmed = trimmed.slice(0, -1)
  return `${trimmed} s`
}

/** Camera gain, always two decimals (auto-exposure produces long floats). */
export function formatGain(gain: number): string {
  return gain.toFixed(2)
}

/** Human-readable file/folder size, e.g. "8.4 MB" or "512 KB". */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const kb = bytes / 1024
  if (kb < 1024) return `${kb.toFixed(1)} KB`
  const mb = kb / 1024
  if (mb < 1024) return `${mb.toFixed(1)} MB`
  return `${(mb / 1024).toFixed(2)} GB`
}

export function formatUptime(sec: number): string {
  const d = Math.floor(sec / 86400)
  const h = Math.floor((sec % 86400) / 3600)
  const m = Math.floor((sec % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}
