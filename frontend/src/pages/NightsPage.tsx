import { useEffect, useState } from 'react'
import { Link } from 'react-router-dom'
import { getApi } from '../api/client'
import type { ArtifactState, NightSummary } from '../api/types'

function ArtifactBadge({ label, artifact }: { label: string; artifact: ArtifactState }) {
  const cls = {
    ready: 'text-ok border-ok/40',
    generating: 'text-warn border-warn/40',
    error: 'text-danger border-danger/40',
    pending: 'text-fgdim border-accent/30',
    disabled: 'text-fgdim border-line',
  }[artifact.state]
  return (
    <span title={`${label}: ${artifact.state}`}
      className={`rounded border px-1 font-mono text-[10px] ${cls}`}>
      {label}
    </span>
  )
}

export default function NightsPage() {
  const [nights, setNights] = useState<NightSummary[] | null>(null)
  const [error, setError] = useState('')
  useEffect(() => {
    void getApi().getNights().then(setNights).catch((e: unknown) => setError(String(e)))
  }, [])

  if (error) return <p className="text-danger">{error}</p>
  if (!nights) return <p className="text-fgdim">Loading…</p>

  return (
    <div>
      <h1 className="mb-4 text-lg font-medium">Nights</h1>
      <div className="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-4">
        {nights.map((n) => (
          <Link key={n.date} to={`/nights/${n.date}`}
            className="overflow-hidden rounded-xl border border-line bg-panel transition hover:border-accent">
            <img src={n.thumbnailUrl} alt={`Night of ${n.date}`}
              className="aspect-square w-full object-cover" />
            <div className="p-3">
              <div className="font-mono text-sm">{n.date}</div>
              <div className="text-xs text-fgdim">{n.frameCount} frames</div>
              <div className="mt-1.5 flex gap-1">
                <ArtifactBadge label="K" artifact={n.keogram} />
                <ArtifactBadge label="S" artifact={n.startrails} />
                <ArtifactBadge label="T" artifact={n.timelapse} />
              </div>
            </div>
          </Link>
        ))}
      </div>
    </div>
  )
}
