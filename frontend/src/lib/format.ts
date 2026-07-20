export function formatExposure(us: number): string {
  const s = us / 1e6
  if (s >= 1) return `${s % 1 === 0 ? s : s.toFixed(1)} s`
  return `1/${Math.round(1 / s)} s`
}

/** Camera gain, always two decimals (auto-exposure produces long floats). */
export function formatGain(gain: number): string {
  return gain.toFixed(2)
}

export function formatUptime(sec: number): string {
  const d = Math.floor(sec / 86400)
  const h = Math.floor((sec % 86400) / 3600)
  const m = Math.floor((sec % 3600) / 60)
  if (d > 0) return `${d}d ${h}h`
  if (h > 0) return `${h}h ${m}m`
  return `${m}m`
}
