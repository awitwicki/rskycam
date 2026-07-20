import { cleanup, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import LocationPicker from './LocationPicker'

type ClickEvent = { latlng: { lat: number; lng: number; wrap: () => { lat: number; lng: number } } }

const state = vi.hoisted(() => ({
  clickHandler: null as ((e: ClickEvent) => void) | null,
  markerPositions: [] as unknown[],
}))

vi.mock('leaflet', () => {
  const map = {
    setView: () => map,
    on: (_ev: string, cb: (e: ClickEvent) => void) => {
      state.clickHandler = cb
    },
    invalidateSize: () => {},
    remove: () => {},
  }
  return {
    map: () => map,
    tileLayer: () => ({ addTo: () => {} }),
    divIcon: () => ({}),
    marker: () => ({
      addTo() {
        return this
      },
      setLatLng(ll: unknown) {
        state.markerPositions.push(ll)
      },
    }),
  }
})
vi.mock('leaflet/dist/leaflet.css', () => ({}))

beforeEach(() => {
  cleanup()
  state.clickHandler = null
  state.markerPositions.length = 0
})

describe('LocationPicker', () => {
  it('reports clicks rounded to 4 decimals with wrapped longitude', async () => {
    const onPick = vi.fn()
    render(<LocationPicker latitudeDeg={50.45} longitudeDeg={30.52} onPick={onPick} />)
    expect(screen.getByTestId('location-map')).toBeInTheDocument()
    await waitFor(() => expect(state.clickHandler).not.toBeNull())
    state.clickHandler!({
      latlng: { lat: 48.858093, lng: 362.294694, wrap: () => ({ lat: 48.858093, lng: 2.294694 }) },
    })
    expect(onPick).toHaveBeenCalledWith(48.8581, 2.2947)
    expect(state.markerPositions).toContainEqual({ lat: 48.858093, lng: 2.294694 })
  })

  it('moves the marker when the coordinates props change', async () => {
    const { rerender } = render(
      <LocationPicker latitudeDeg={50} longitudeDeg={30} onPick={() => {}} />,
    )
    await waitFor(() => expect(state.clickHandler).not.toBeNull())
    rerender(<LocationPicker latitudeDeg={10.5} longitudeDeg={20.25} onPick={() => {}} />)
    expect(state.markerPositions).toContainEqual([10.5, 20.25])
  })
})
