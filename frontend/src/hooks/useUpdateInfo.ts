import { useEffect, useState } from 'react'
import { getApi } from '../api/client'
import type { UpdateInfo } from '../api/types'

// One fetch per session, shared between the sidebar widget and the
// mobile header dot; the server caches the GitHub check anyway.
let cache: UpdateInfo | null = null
let inflight: Promise<UpdateInfo> | null = null

export function useUpdateInfo(): UpdateInfo | null {
  const [info, setInfo] = useState<UpdateInfo | null>(cache)
  useEffect(() => {
    if (cache) return
    inflight ??= getApi().getUpdate()
    let alive = true
    inflight
      .then((i) => {
        cache = i
        if (alive) setInfo(i)
      })
      .catch(() => {}) // no version line is fine; never crash the layout
    return () => {
      alive = false
    }
  }, [])
  return info
}

export function _resetUpdateInfoForTests() {
  cache = null
  inflight = null
}
