import { act, cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { MemoryRouter, Route, Routes } from 'react-router-dom'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { setApi } from '../api/client'
import type { ApiClient } from '../api/client'
import type { NightDetail } from '../api/types'
import NightDetailPage from './NightDetailPage'

const night = (timelapse: NightDetail['timelapse']): NightDetail => ({
  date: '2026-07-14',
  frameCount: 2,
  thumbnailUrl: '',
  keogram: { state: 'pending' },
  startrails: { state: 'pending' },
  timelapse,
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
      .mockResolvedValue(night({ state: 'ready', url: '/api/files/x/timelapse.mp4' }))
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
      .mockResolvedValue(night({ state: 'ready', url: '/x/timelapse.mp4' }))
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

describe('NightDetailPage keogram rendering', () => {
  afterEach(cleanup)

  it('renders the keogram as a fixed-height strip, not aspect-scaled', async () => {
    // A keogram is width=frames × height=frame-height, i.e. portrait for
    // most of the night (424×1232 real example). With plain w-full the
    // browser scales height by aspect ratio and the image becomes several
    // thousand px tall. It must render as a fixed-height stretched strip.
    const ready: NightDetail = {
      ...night({ state: 'pending' }),
      keogram: { state: 'ready', url: '/api/files/2026-07-14/keogram.jpg' },
    }
    setApi({ getNight: () => Promise.resolve(ready) } as unknown as ApiClient)

    renderPage()
    const img = await screen.findByAltText('Keogram')
    expect(img.className).toMatch(/\bh-44\b/) // fixed strip height
    expect(img.className).not.toMatch(/object-contain|object-cover/) // stretch to fill
  })
})
