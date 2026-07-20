import type { ReactNode } from 'react'
import type { ArtifactState } from '../api/types'
import { Card } from './ui'

export default function ArtifactCard({ title, artifact, children }: {
  title: string
  artifact: ArtifactState
  children: (url: string) => ReactNode
}) {
  return (
    <Card title={title}
      action={artifact.state === 'ready' ? (
        <a href={artifact.url} download className="text-xs text-accent hover:underline">
          Download
        </a>
      ) : undefined}>
      {artifact.state === 'ready' && children(artifact.url)}
      {artifact.state === 'generating' && <p className="animate-pulse text-sm text-warn">Generating…</p>}
      {artifact.state === 'error' && <p className="text-sm text-danger">Failed: {artifact.message}</p>}
      {artifact.state === 'pending' && <p className="text-sm text-fgdim">Not generated yet</p>}
      {artifact.state === 'disabled' && <p className="text-sm text-fgdim">Disabled in settings</p>}
    </Card>
  )
}
