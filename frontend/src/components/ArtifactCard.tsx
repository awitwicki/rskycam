import type { ReactNode } from 'react'
import type { ArtifactState } from '../api/types'
import { formatBytes } from '../lib/format'
import { Card } from './ui'

export default function ArtifactCard({ title, artifact, showSize, children }: {
  title: string
  artifact: ArtifactState
  showSize?: boolean
  children: (url: string) => ReactNode
}) {
  return (
    <Card title={title}
      action={artifact.state === 'ready' ? (
        <span className="flex items-center gap-2">
          {showSize && <span className="text-xs text-fgdim">{formatBytes(artifact.sizeBytes)}</span>}
          <a href={artifact.url} download className="text-xs text-accent hover:underline">
            Download
          </a>
        </span>
      ) : undefined}>
      {artifact.state === 'ready' && children(artifact.url)}
      {artifact.state === 'generating' && <p className="animate-pulse text-sm text-warn">Generating…</p>}
      {artifact.state === 'error' && <p className="text-sm text-danger">Failed: {artifact.message}</p>}
      {artifact.state === 'skipped' && <p className="text-sm text-fgdim">Skipped: {artifact.message}</p>}
      {artifact.state === 'pending' && <p className="text-sm text-fgdim">Not generated yet</p>}
      {artifact.state === 'disabled' && <p className="text-sm text-fgdim">Disabled in settings</p>}
    </Card>
  )
}
