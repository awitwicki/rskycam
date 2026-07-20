import { describe, it, expect } from 'vitest'
import { formatExposure, formatUptime } from './format'

describe('formatExposure', () => {
  it('formats whole seconds', () => expect(formatExposure(30_000_000)).toBe('30 s'))
  it('formats fractional seconds', () => expect(formatExposure(2_500_000)).toBe('2.5 s'))
  it('formats sub-second as 1/x', () => expect(formatExposure(2_000)).toBe('1/500 s'))
})

describe('formatUptime', () => {
  it('days+hours', () => expect(formatUptime(3 * 86400 + 4 * 3600 + 120)).toBe('3d 4h'))
  it('hours+minutes', () => expect(formatUptime(2 * 3600 + 300)).toBe('2h 5m'))
  it('minutes only', () => expect(formatUptime(240)).toBe('4m'))
})
