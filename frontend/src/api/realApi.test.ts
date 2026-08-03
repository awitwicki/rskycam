import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RealApi } from './realApi'

const okJson = (data: unknown) =>
  new Response(JSON.stringify(data), { status: 200, headers: { 'Content-Type': 'application/json' } })

let fetchMock: ReturnType<typeof vi.fn>

beforeEach(() => {
  localStorage.clear()
  fetchMock = vi.fn()
  vi.stubGlobal('fetch', fetchMock)
})

afterEach(() => vi.unstubAllGlobals())

describe('RealApi', () => {
  it('login stores the auth flag on success and not on failure', async () => {
    const api = new RealApi()
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 401 }))
    expect(await api.login('admin', 'bad')).toBe(false)
    expect(api.isAuthenticated()).toBe(false)
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))
    expect(await api.login('admin', 'good')).toBe(true)
    expect(api.isAuthenticated()).toBe(true)
  })

  it('a 401 clears the session flag and dispatches rskycam:unauthorized', async () => {
    localStorage.setItem('rskycam.auth', '1')
    const api = new RealApi()
    const seen = vi.fn()
    window.addEventListener('rskycam:unauthorized', seen)
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 401 }))
    await expect(api.getStatus()).rejects.toThrow(/unauthorized/)
    expect(api.isAuthenticated()).toBe(false)
    expect(seen).toHaveBeenCalledOnce()
    window.removeEventListener('rskycam:unauthorized', seen)
  })

  it('getOverlay posts the request preserving crop tri-state', async () => {
    const api = new RealApi()
    // mockImplementation (not mockResolvedValue) so each call gets a fresh Response —
    // a real Response body can only be read once, and getOverlay reads it via .json().
    fetchMock.mockImplementation(() =>
      Promise.resolve(okJson({ imageWidth: 1, imageHeight: 1, polylines: [], labels: [] })))
    await api.getOverlay({ crop: null })
    const body1 = JSON.parse((fetchMock.mock.calls[0][1] as RequestInit).body as string)
    expect(body1.crop).toBeNull()
    await api.getOverlay({})
    const body2 = JSON.parse((fetchMock.mock.calls[1][1] as RequestInit).body as string)
    expect('crop' in body2).toBe(false)
  })

  it('rebuildNight rejects on 404; latestImageUrl carries the raw flag', async () => {
    const api = new RealApi()
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 404 }))
    await expect(api.rebuildNight('1999-01-01')).rejects.toThrow(/404/)
    expect(api.latestImageUrl({ raw: true })).toContain('raw=1')
    expect(api.latestImageUrl()).toContain('raw=0')
  })

  it('setFocus posts the body and focus URLs are cache-busted', async () => {
    const api = new RealApi()
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))
    await api.setFocus(true, 500_000)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/focus')
    const body = JSON.parse((fetchMock.mock.calls[0][1] as RequestInit).body as string)
    expect(body).toEqual({ enabled: true, exposureUs: 500_000 })
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 204 }))
    await api.setFocus(false)
    const body2 = JSON.parse((fetchMock.mock.calls[1][1] as RequestInit).body as string)
    expect(body2).toEqual({ enabled: false })
    expect(api.focusImageUrl()).toContain('/api/focus.jpg?ts=')
    expect(api.focusStarUrl()).toContain('/api/focus/star.png?ts=')
  })

  it('getUpdate parses the update info and applyUpdate posts', async () => {
    const api = new RealApi()
    fetchMock.mockResolvedValueOnce(
      okJson({ current: '0.5.0.7', latest: 'v0.5.0.9', updateAvailable: true, error: null }))
    const info = await api.getUpdate()
    expect(info.updateAvailable).toBe(true)
    expect(fetchMock.mock.calls[0][0]).toBe('/api/update')

    fetchMock.mockResolvedValueOnce(new Response(null, { status: 202 }))
    await api.applyUpdate()
    expect(fetchMock.mock.calls[1][0]).toBe('/api/update/apply')
    expect((fetchMock.mock.calls[1][1] as RequestInit).method).toBe('POST')
  })
})
