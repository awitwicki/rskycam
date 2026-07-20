import { RefreshCw, Trash2 } from 'lucide-react'
import { useCallback, useEffect, useState } from 'react'
import { useNavigate, useParams } from 'react-router-dom'
import { getApi } from '../api/client'
import type { NightDetail } from '../api/types'
import ArtifactCard from '../components/ArtifactCard'
import Lightbox, { type LightboxItem } from '../components/Lightbox'
import { Button, Card } from '../components/ui'
import { formatExposure, formatGain } from '../lib/format'

export default function NightDetailPage() {
  const { date = '' } = useParams()
  const navigate = useNavigate()
  const [night, setNight] = useState<NightDetail | null>(null)
  const [error, setError] = useState('')
  const [confirmDelete, setConfirmDelete] = useState(false)
  const [deleting, setDeleting] = useState(false)
  const [lightbox, setLightbox] = useState<{ items: LightboxItem[]; index: number } | null>(null)

  const load = useCallback(() => {
    getApi().getNight(date).then(setNight).catch((e: unknown) => setError(String(e)))
  }, [date])

  useEffect(load, [load])

  const anyGenerating =
    night !== null &&
    [night.keogram, night.startrails, night.timelapse].some((a) => a.state === 'generating')

  // A rebuild (or dawn) leaves artifacts in 'generating'; poll until they settle.
  useEffect(() => {
    if (!anyGenerating) return
    const t = setInterval(load, 5000)
    return () => clearInterval(t)
  }, [anyGenerating, load])

  const rebuild = async () => {
    try {
      await getApi().rebuildNight(date)
      load()
    } catch (e: unknown) {
      setError(String(e))
    }
  }

  const remove = async () => {
    setDeleting(true)
    try {
      await getApi().deleteNight(date)
      navigate('/nights')
    } catch (e: unknown) {
      setError(String(e))
      setDeleting(false)
      setConfirmDelete(false)
    }
  }

  if (error) return <p className="text-danger">{error}</p>
  if (!night) return <p className="text-fgdim">Loading…</p>

  return (
    <div className="flex flex-col gap-4">
      <header className="flex items-center justify-between">
        <h1 className="font-mono text-lg">
          {night.date}
          <span className="ml-3 text-sm text-fgdim">{night.frameCount} frames</span>
        </h1>
        <div className="flex items-center gap-2">
          <Button variant="ghost" onClick={rebuild}>
            <RefreshCw size={14} /> Rebuild
          </Button>
          <Button variant="ghost" onClick={() => setConfirmDelete(true)}
            className="text-danger hover:border-danger">
            <Trash2 size={14} /> Delete
          </Button>
        </div>
      </header>

      {confirmDelete && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-night/80 p-4"
          role="dialog" aria-modal="true" aria-label="Confirm delete night"
          onClick={() => !deleting && setConfirmDelete(false)}>
          <div className="w-full max-w-sm rounded-xl border border-line bg-panel p-5"
            onClick={(e) => e.stopPropagation()}>
            <h2 className="font-mono text-base">Delete night {night.date}?</h2>
            <p className="mt-2 text-sm text-fgdim">
              This permanently removes all {night.frameCount} frames and the keogram,
              star trails and timelapse for this night. This can’t be undone.
            </p>
            <div className="mt-5 flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirmDelete(false)} disabled={deleting}>
                Cancel
              </Button>
              <Button variant="danger" onClick={remove} disabled={deleting}>
                {deleting ? 'Deleting…' : 'Delete night'}
              </Button>
            </div>
          </div>
        </div>
      )}

      <ArtifactCard title="Keogram" artifact={night.keogram}>
        {(url) => (
          <button className="block w-full" aria-label="View keogram fullscreen"
            onClick={() => setLightbox({ items: [{ url, caption: `Keogram · ${night.date}`, downloadName: `keogram-${night.date}.jpg` }], index: 0 })}>
            {/* One column per frame × full frame height makes the natural
                aspect portrait for most of the night; keograms are time
                strips, so stretch into a fixed-height bar instead of
                aspect-scaling (the lightbox still shows the true aspect). */}
            <img src={url} alt="Keogram" className="h-44 w-full rounded-lg" />
          </button>
        )}
      </ArtifactCard>

      <div className="grid gap-4 md:grid-cols-2">
        <ArtifactCard title="Star trails" artifact={night.startrails}>
          {(url) => (
            <button className="block w-full" aria-label="View star trails fullscreen"
              onClick={() => setLightbox({ items: [{ url, caption: `Star trails · ${night.date}`, downloadName: `startrails-${night.date}.jpg` }], index: 0 })}>
              <img src={url} alt="Star trails" className="w-full rounded-lg" />
            </button>
          )}
        </ArtifactCard>
        <ArtifactCard title="Timelapse" artifact={night.timelapse}>
          {(url) => <video src={url} controls className="w-full rounded-lg" />}
        </ArtifactCard>
      </div>

      <Card title="Frames">
        <div className="grid grid-cols-3 gap-2 sm:grid-cols-4 lg:grid-cols-6">
          {night.frames.map((f, i) => (
            <button key={f.timestamp}
              title={`${new Date(f.timestamp).toLocaleTimeString()} · exp ${formatExposure(f.exposureUs)} · gain ${formatGain(f.gain)}`}
              aria-label={`View frame ${new Date(f.timestamp).toLocaleTimeString()}`}
              onClick={() => setLightbox({
                items: night.frames.map((x) => ({
                  url: x.url,
                  caption: `${new Date(x.timestamp).toLocaleString()} · exp ${formatExposure(x.exposureUs)} · gain ${formatGain(x.gain)}`,
                  downloadName: `frame-${x.timestamp.replace(/[:.]/g, '-')}.jpg`,
                })),
                index: i,
              })}>
              <img src={f.url} alt={`Frame ${new Date(f.timestamp).toLocaleTimeString()}`}
                className="aspect-square w-full rounded object-cover transition hover:opacity-80" />
            </button>
          ))}
        </div>
      </Card>

      {lightbox && (
        <Lightbox items={lightbox.items} index={lightbox.index}
          onClose={() => setLightbox(null)}
          onNavigate={(i) => setLightbox({ ...lightbox, index: i })} />
      )}
    </div>
  )
}
