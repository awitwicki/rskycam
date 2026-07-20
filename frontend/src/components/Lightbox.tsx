import { ChevronLeft, ChevronRight, X } from 'lucide-react'
import { useEffect } from 'react'

export interface LightboxItem {
  url: string
  caption?: string
  downloadName?: string
}

export default function Lightbox({ items, index, onClose, onNavigate }: {
  items: LightboxItem[]
  index: number
  onClose: () => void
  onNavigate?: (nextIndex: number) => void
}) {
  const item = items[index] as LightboxItem | undefined
  const hasPrev = index > 0
  const hasNext = index < items.length - 1

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose()
      if (e.key === 'ArrowLeft' && hasPrev) onNavigate?.(index - 1)
      if (e.key === 'ArrowRight' && hasNext) onNavigate?.(index + 1)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [index, hasPrev, hasNext, onClose, onNavigate])

  if (!item) return null

  return (
    <div role="dialog" aria-modal="true" aria-label="Image viewer"
      className="fixed inset-0 z-50 flex flex-col bg-night/95" onClick={onClose}>
      <div className="flex items-center justify-between gap-3 p-3"
        onClick={(e) => e.stopPropagation()}>
        <span className="truncate font-mono text-sm text-fgdim">{item.caption}</span>
        <div className="flex shrink-0 items-center gap-4">
          <a href={item.url} download={item.downloadName}
            className="text-sm text-accent hover:underline">
            Download
          </a>
          <button aria-label="Close" onClick={onClose} className="text-fgdim hover:text-fg">
            <X size={22} />
          </button>
        </div>
      </div>

      <div className="relative flex flex-1 items-center justify-center overflow-hidden p-3 pb-6">
        <img src={item.url} alt={item.caption ?? 'Frame'}
          className="max-h-full max-w-full rounded object-contain"
          onClick={(e) => e.stopPropagation()} />
        {onNavigate && hasPrev && (
          <button aria-label="Previous image"
            onClick={(e) => { e.stopPropagation(); onNavigate(index - 1) }}
            className="absolute left-3 rounded-full border border-line bg-panel/90 p-2 text-fg hover:bg-panel2">
            <ChevronLeft size={22} />
          </button>
        )}
        {onNavigate && hasNext && (
          <button aria-label="Next image"
            onClick={(e) => { e.stopPropagation(); onNavigate(index + 1) }}
            className="absolute right-3 rounded-full border border-line bg-panel/90 p-2 text-fg hover:bg-panel2">
            <ChevronRight size={22} />
          </button>
        )}
        {items.length > 1 && (
          <span className="absolute bottom-1 rounded bg-panel/90 px-2 py-0.5 font-mono text-xs text-fgdim">
            {index + 1} / {items.length}
          </span>
        )}
      </div>
    </div>
  )
}
