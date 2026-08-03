import { useState } from 'react'
import { getApi } from '../api/client'
import { HttpError } from '../api/realApi'
import { useUpdateInfo } from '../hooks/useUpdateInfo'

type Phase = 'idle' | 'confirm' | 'working' | 'done' | 'failed'

/** Sidebar version line; becomes an "Update" pill when a newer GitHub
 * release exists, with a confirm dialog driving the staged self-update. */
export default function UpdateWidget() {
  const info = useUpdateInfo()
  const [phase, setPhase] = useState<Phase>('idle')
  const [newVersion, setNewVersion] = useState<string | null>(null)
  const [failureMessage, setFailureMessage] = useState<string | null>(null)

  if (!info) return null

  async function runUpdate() {
    if (!info) return
    setPhase('working')
    try {
      await getApi().applyUpdate()
    } catch (e) {
      if (e instanceof HttpError) {
        // A real HTTP-level rejection (409 no newer release, 502 staging
        // failed, 503 hook missing) — the server never restarted, so
        // there's nothing to poll for. Surface its message immediately
        // instead of waiting out the full timeout.
        setFailureMessage(e.body || `Update rejected (HTTP ${e.status})`)
        setPhase('failed')
        return
      }
      // A network-level failure (connection dropped, not an HTTP
      // response) — the server likely already exited to apply the
      // update. Keep polling.
    }
    const deadline = Date.now() + 120_000
    while (Date.now() < deadline) {
      await new Promise((r) => setTimeout(r, 3000))
      try {
        const s = await getApi().getStatus()
        if (s.version !== info.current) {
          setNewVersion(s.version)
          setPhase('done')
          return
        }
      } catch {
        // still restarting
      }
    }
    setPhase('failed')
  }

  return (
    <>
      {info.updateAvailable && info.latest ? (
        <button
          onClick={() => {
            setFailureMessage(null) // a retry shouldn't show a stale failure
            setPhase('confirm')
          }}
          className="flex items-center gap-2 rounded-lg px-3 py-1 text-left text-xs text-accent hover:bg-panel2"
        >
          <span className="h-1.5 w-1.5 rounded-full bg-accent" />
          Update → {info.latest}
        </button>
      ) : (
        <div className="px-3 py-1 font-mono text-xs text-fgdim">v{info.current}</div>
      )}

      {phase !== 'idle' && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div className="w-80 rounded-xl border border-line bg-panel p-4">
            {phase === 'confirm' && (
              <>
                <div className="mb-2 text-sm">Update rskycam?</div>
                <div className="mb-4 font-mono text-xs text-fgdim">
                  v{info.current} → {info.latest}
                </div>
                <div className="flex justify-end gap-2">
                  <button onClick={() => setPhase('idle')}
                    className="rounded-lg px-3 py-1.5 text-sm text-fgdim hover:text-fg">
                    Cancel
                  </button>
                  <button onClick={() => void runUpdate()}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-black">
                    Update
                  </button>
                </div>
              </>
            )}
            {phase === 'working' && (
              <div className="text-sm text-fgdim">
                Installing update — the service is restarting, this page will
                recover in under a minute…
              </div>
            )}
            {phase === 'done' && (
              <>
                <div className="mb-4 text-sm">Updated to v{newVersion} ✓</div>
                <div className="flex justify-end">
                  <button onClick={() => window.location.assign('/')}
                    className="rounded-lg bg-accent px-3 py-1.5 text-sm text-black">
                    Reload
                  </button>
                </div>
              </>
            )}
            {phase === 'failed' && (
              <>
                <div className="mb-4 text-sm">
                  {failureMessage ?? (
                    <>
                      The service did not come back with a new version within 2
                      minutes. Check <span className="font-mono">journalctl -u rskycam</span>.
                    </>
                  )}
                </div>
                <div className="flex justify-end">
                  <button onClick={() => setPhase('idle')}
                    className="rounded-lg px-3 py-1.5 text-sm text-fgdim hover:text-fg">
                    Close
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}
    </>
  )
}
