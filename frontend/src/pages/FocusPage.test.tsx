import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { setApi, type ApiClient } from '../api/client'
import { MockApi } from '../api/mock/mockApi'
import type { ApiEvent, FocusMeta } from '../api/types'
import FocusPage from './FocusPage'

beforeEach(() => {
  cleanup()
  localStorage.clear()
})

const meta = (over: Partial<FocusMeta> = {}): FocusMeta => ({
  timestamp: '2026-08-01T22:00:00Z', hfd: 4.25, starX: 512, starY: 384,
  peak: 213, saturated: false, exposureUs: 1_000_000, gain: 8, ...over,
})

/** MockApi with focus spies and a manually-driven event stream. */
function focusApi() {
  const base = new MockApi({ renderFrame: () => 'data:,x' })
  let emit: (e: ApiEvent) => void = () => {}
  const setFocus = vi.fn(async () => {})
  const api = {
    ...Object.fromEntries(
      ['login', 'logout', 'getLightgraph', 'getLogs', 'getOverlay', 'getSettings',
       'putSettings', 'changePassword', 'getNights', 'getNight', 'rebuildNight',
       'deleteNight', 'startDarksCapture', 'getDarksLibrary', 'clearDarks',
      ].map((k) => [k, (base as unknown as Record<string, (...a: unknown[]) => unknown>)[k].bind(base)]),
    ),
    isAuthenticated: () => true,
    getStatus: base.getStatus.bind(base),
    latestImageUrl: () => 'data:,x',
    focusImageUrl: () => 'data:,crop',
    focusStarUrl: () => 'data:,star',
    setFocus,
    subscribe: (cb: (e: ApiEvent) => void) => {
      emit = cb
      return () => {}
    },
  } as unknown as ApiClient
  return { api, setFocus, emit: (e: ApiEvent) => emit(e) }
}

describe('FocusPage', () => {
  it('start button posts enabled with the default exposure and gain', async () => {
    const { api, setFocus } = focusApi()
    setApi(api)
    render(<FocusPage />)
    fireEvent.click(await screen.findByRole('button', { name: /start/i }))
    expect(setFocus).toHaveBeenCalledWith(true, 1_000_000, expect.any(Number))
  })

  it('a focus event shows HFD and appends a chart sample', async () => {
    const { api, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    await screen.findByRole('button', { name: /start/i })
    emit({ type: 'focus', meta: meta() })
    await waitFor(() => expect(screen.getByText('4.25')).toBeInTheDocument())
    expect(document.querySelector('polyline')).toBeTruthy()
  })

  it('saturated star shows the badge; hfd null shows no-star note', async () => {
    const { api, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    await screen.findByRole('button', { name: /start/i })
    emit({ type: 'focus', meta: meta({ saturated: true }) })
    await waitFor(() => expect(screen.getByText(/saturated/i)).toBeInTheDocument())
    emit({ type: 'focus', meta: meta({ hfd: null }) })
    await waitFor(() => expect(screen.getByText(/no star/i)).toBeInTheDocument())
  })

  it('changing the exposure select while running re-posts with the new value', async () => {
    const { api, setFocus, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    fireEvent.click(await screen.findByRole('button', { name: /start/i }))
    emit({ type: 'focus', meta: meta() }) // running now
    fireEvent.change(screen.getByLabelText(/exposure/i), { target: { value: '500000' } })
    await waitFor(() => expect(setFocus).toHaveBeenLastCalledWith(true, 500_000, expect.any(Number)))
  })

  it('changing the gain select while running re-posts with the new value', async () => {
    const { api, setFocus, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    fireEvent.click(await screen.findByRole('button', { name: /start/i }))
    emit({ type: 'focus', meta: meta() }) // running now
    const gainSelect = screen.getByLabelText(/gain/i) as HTMLSelectElement
    const otherValue = Array.from(gainSelect.options)
      .map((o) => o.value)
      .find((v) => v !== gainSelect.value)!
    fireEvent.change(gainSelect, { target: { value: otherValue } })
    await waitFor(() => expect(setFocus).toHaveBeenLastCalledWith(true, 1_000_000, Number(otherValue)))
  })

  it('shows the full uncropped preview, not a fixed server-side crop', async () => {
    const { api, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    await screen.findByRole('button', { name: /start/i })
    emit({ type: 'focus', meta: meta() })
    await waitFor(() => expect(screen.getByAltText(/zoom/i)).toBeInTheDocument())
    expect(screen.queryByAltText(/crop/i)).not.toBeInTheDocument()
  })

  it('scrolling the preview zooms in, and reset zoom returns to 1x', async () => {
    const { api, emit } = focusApi()
    setApi(api)
    render(<FocusPage />)
    await screen.findByRole('button', { name: /start/i })
    emit({ type: 'focus', meta: meta() })
    const img = await screen.findByAltText(/zoom/i)
    expect(screen.queryByRole('button', { name: /reset zoom/i })).not.toBeInTheDocument()

    fireEvent.wheel(img.parentElement!, { deltaY: -200, clientX: 50, clientY: 50 })
    const resetButton = await screen.findByRole('button', { name: /reset zoom/i })

    fireEvent.click(resetButton)
    await waitFor(() => expect(screen.queryByRole('button', { name: /reset zoom/i })).not.toBeInTheDocument())
  })
})
