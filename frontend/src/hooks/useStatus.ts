import { useEffect, useState } from 'react'
import { getApi } from '../api/client'
import type { FrameMeta, Status } from '../api/types'

export function useStatus() {
  const [status, setStatus] = useState<Status | null>(null)
  const [frame, setFrame] = useState<{ url: string; meta: FrameMeta } | null>(null)

  useEffect(() => {
    const api = getApi()
    void api.getStatus().then(setStatus)
    return api.subscribe((e) => {
      if (e.type === 'status') setStatus(e.status)
      else setFrame({ url: e.imageUrl, meta: e.meta })
    })
  }, [])

  return { status, frame }
}
