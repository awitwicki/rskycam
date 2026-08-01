import { afterEach, describe, expect, it, vi } from 'vitest'
import { uid } from './uid'

const V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/

afterEach(() => vi.unstubAllGlobals())

describe('uid', () => {
  it('returns a v4 uuid', () => {
    expect(uid()).toMatch(V4)
    expect(uid()).not.toBe(uid())
  })

  it('works without crypto.randomUUID (plain-http Pi is not a secure context)', () => {
    const real = globalThis.crypto
    vi.stubGlobal('crypto', { getRandomValues: real.getRandomValues.bind(real) })
    expect('randomUUID' in crypto).toBe(false)
    expect(uid()).toMatch(V4)
    expect(uid()).not.toBe(uid())
  })
})
