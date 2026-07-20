import { act, renderHook } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { useDebouncedValue } from './useDebouncedValue'

afterEach(() => vi.useRealTimers())

describe('useDebouncedValue', () => {
  it('only updates after the delay', () => {
    vi.useFakeTimers()
    const { result, rerender } = renderHook(({ v }) => useDebouncedValue(v, 150), {
      initialProps: { v: 1 },
    })
    rerender({ v: 2 })
    expect(result.current).toBe(1)
    act(() => vi.advanceTimersByTime(200))
    expect(result.current).toBe(2)
  })
})
