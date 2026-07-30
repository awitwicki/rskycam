import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setApi } from '../api/client'
import type { ApiClient } from '../api/client'
import type { NightDetail } from '../api/types'
import NightDetailPage from './NightDetailPage'

const night = (timelapseNight: NightDetail['timelapseNight']): NightDetail => ({
  date: '2026-07-14',
  frameCount: 2,
  framesSizeBytes: 500_000,
  totalSizeBytes: 500_000,
  thumbnailUrl: '',
  keogram: { state: 'pending' },
  startrails: { state: 'pending' },
  timelapseDay: { state: 'pending' },
  timelapseNight,
  frames: [],
})

function renderPage() {
  return render(
    <MemoryRouter initialEntries={['/nights/2026-07-14']}>
      <Routes>
        <Route path="/nights/:date" element={<NightDetailPage />} />
      </Routes>
    </MemoryRouter>,
  )
}

describe('NightDetailPage generating poll', () => {
  beforeEach(() => vi.useFakeTimers({ shouldAdvanceTime: true }))
  afterEach(() => {
    vi.useRealTimers()
    cleanup()
  })

  it('refetches every 5s while generating, then stops', async () => {
    const getNight = vi
      .fn<() => Promise<NightDetail>>()
      .mockResolvedValueOnce(night({ state: 'generating' }))
      .mockResolvedValueOnce(night({ state: 'generating' }))
      .mockResolvedValue(night({ state: 'ready', url: '/api/files/x/timelapse.mp4', sizeBytes: 12_000_000 }))
    setApi({ getNight } as unknown as ApiClient)

    renderPage()

    // Wait for initial load to complete
    await act(async () => {})

    expect(await screen.findByText(/generating/i)).toBeInTheDocument()
    expect(getNight).toHaveBeenCalledTimes(1)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000)
    })
    expect(getNight).toHaveBeenCalledTimes(2)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000)
    })
    expect(getNight).toHaveBeenCalledTimes(3)
    expect(screen.queryByText(/generating/i)).not.toBeInTheDocument()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(15000)
    })
    expect(getNight).toHaveBeenCalledTimes(3) // stopped polling
  }, 15000)
})

describe('NightDetailPage delete', () => {
  afterEach(cleanup)

  it('deletes only after confirmation, then navigates to the list', async () => {
    const deleteNight = vi.fn<(d: string) => Promise<void>>().mockResolvedValue()
    const getNight = vi
      .fn<() => Promise<NightDetail>>()
      .mockResolvedValue(night({ state: 'ready', url: '/x/timelapse.mp4', sizeBytes: 12_000_000 }))
    setApi({ getNight, deleteNight } as unknown as ApiClient)

    render(
      <MemoryRouter initialEntries={['/nights/2026-07-14']}>
        <Routes>
          <Route path="/nights/:date" element={<NightDetailPage />} />
          <Route path="/nights" element={<div>NIGHTS LIST</div>} />
        </Routes>
      </MemoryRouter>,
    )

    // Clicking Delete only opens the confirm dialog — a misclick deletes nothing.
    await userEvent.click(await screen.findByRole('button', { name: /^delete$/i }))
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(deleteNight).not.toHaveBeenCalled()

    // Cancel closes the dialog without deleting.
    await userEvent.click(screen.getByRole('button', { name: /cancel/i }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(deleteNight).not.toHaveBeenCalled()

    // Confirming actually deletes and routes back to the gallery.
    await userEvent.click(screen.getByRole('button', { name: /^delete$/i }))
    await userEvent.click(screen.getByRole('button', { name: /delete night/i }))
    expect(deleteNight).toHaveBeenCalledWith('2026-07-14')
    expect(await screen.findByText('NIGHTS LIST')).toBeInTheDocument()
  })
})

describe('NightDetailPage frame grid', () => {
  afterEach(cleanup)

  it('shows thumbnails in the grid but opens the full image on click', async () => {
    const ready: NightDetail = {
      ...night({ state: 'pending' }),
      frames: [{
        timestamp: '2026-07-14T22:00:00Z',
        url: '/api/files/2026-07-14/frames/full.jpg',
        thumbUrl: '/api/files/2026-07-14/frames/full.jpg?thumb=1',
        exposureUs: 30_000_000,
        gain: 8,
      }],
    }
    setApi({ getNight: () => Promise.resolve(ready) } as unknown as ApiClient)

    renderPage()
    const gridImg = await screen.findByAltText(/^frame /i)
    expect(gridImg.getAttribute('src')).toBe('/api/files/2026-07-14/frames/full.jpg?thumb=1')

    await userEvent.click(gridImg)
    const dialog = await screen.findByRole('dialog')
    const fullImg = dialog.querySelector('img')
    expect(fullImg?.getAttribute('src')).toBe('/api/files/2026-07-14/frames/full.jpg')
  })
})

describe('NightDetailPage keogram rendering', () => {
  afterEach(cleanup)

  it('renders the keogram as a fixed-height strip, not aspect-scaled', async () => {
    // A keogram is width=frames × height=frame-height, i.e. portrait for
    // most of the night (424×1232 real example). With plain w-full the
    // browser scales height by aspect ratio and the image becomes several
    // thousand px tall. It must render as a fixed-height stretched strip.
    const ready: NightDetail = {
      ...night({ state: 'pending' }),
      keogram: { state: 'ready', url: '/api/files/2026-07-14/keogram.jpg', sizeBytes: 640_000 },
    }
    setApi({ getNight: () => Promise.resolve(ready) } as unknown as ApiClient)

    renderPage()
    const img = await screen.findByAltText('Keogram')
    expect(img.className).toMatch(/\bh-44\b/) // fixed strip height
    expect(img.className).not.toMatch(/object-contain|object-cover/) // stretch to fill
  })
})
