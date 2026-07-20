import 'leaflet/dist/leaflet.css' // static so the core styles ship in the main bundle
import { useEffect, useRef } from 'react'
import type { Map as LeafletMap, Marker } from 'leaflet'

export interface LocationPickerProps {
  latitudeDeg: number
  longitudeDeg: number
  onPick: (latitudeDeg: number, longitudeDeg: number) => void
}

/** ~11 m — plenty for the sky overlay, keeps the number fields readable. */
const round4 = (v: number) => Math.round(v * 10_000) / 10_000

/**
 * OpenStreetMap click-to-pick location. Leaflet loads on demand so it costs
 * nothing until the map is opened; tiles come from openstreetmap.org and
 * need internet access.
 */
export default function LocationPicker({ latitudeDeg, longitudeDeg, onPick }: LocationPickerProps) {
  const elRef = useRef<HTMLDivElement>(null)
  const mapRef = useRef<LeafletMap | null>(null)
  const markerRef = useRef<Marker | null>(null)
  const propsRef = useRef({ latitudeDeg, longitudeDeg, onPick })
  propsRef.current = { latitudeDeg, longitudeDeg, onPick }

  useEffect(() => {
    let cancelled = false
    let ro: ResizeObserver | undefined
    void (async () => {
      const L = await import('leaflet')
      if (cancelled || !elRef.current || mapRef.current) return
      const start = propsRef.current
      const map = L.map(elRef.current).setView([start.latitudeDeg, start.longitudeDeg], 5)
      L.tileLayer('https://tile.openstreetmap.org/{z}/{x}/{y}.png', {
        maxZoom: 19,
        attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
      }).addTo(map)
      const icon = L.divIcon({
        className: '',
        html: '<span class="block h-3 w-3 rounded-full bg-accent ring-2 ring-fg"></span>',
        iconSize: [12, 12],
        iconAnchor: [6, 6],
      })
      const marker = L.marker([start.latitudeDeg, start.longitudeDeg], { icon }).addTo(map)
      map.on('click', (e) => {
        const ll = e.latlng.wrap() // keep longitude in [-180, 180] after panning around
        marker.setLatLng(ll)
        propsRef.current.onPick(round4(ll.lat), round4(ll.lng))
      })
      mapRef.current = map
      markerRef.current = marker
      // The container gets its size only when the "Pick on map" toggle reveals
      // it, so Leaflet often initializes against a zero/stale box and renders a
      // blank map. Re-measure now and whenever the container resizes.
      map.invalidateSize()
      if (typeof ResizeObserver !== 'undefined') {
        ro = new ResizeObserver(() => map.invalidateSize())
        ro.observe(elRef.current)
      }
    })()
    return () => {
      cancelled = true
      ro?.disconnect()
      mapRef.current?.remove()
      mapRef.current = null
      markerRef.current = null
    }
  }, [])

  // Follow edits made through the numeric fields.
  useEffect(() => {
    markerRef.current?.setLatLng([latitudeDeg, longitudeDeg])
  }, [latitudeDeg, longitudeDeg])

  return (
    <div ref={elRef} data-testid="location-map" aria-label="Pick location on map"
      className="map-dark relative isolate z-0 h-64 w-full overflow-hidden rounded-lg border border-line" />
  )
}
