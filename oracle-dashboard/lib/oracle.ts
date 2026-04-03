import { createPublicClient, http, parseAbi } from 'viem'

export const ORACLE_ADDRESS = '0x0AcC010000000000000000000000000000000000' as const

export const ORACLE_ABI = parseAbi([
  'function getOraclePrice(uint32 currencyId) external view returns (uint256)',
  'function getCurrencies() external view returns (uint32[])',
  'function getPairPrice(uint32 base, uint32 quote) external view returns (uint256)',
])

export interface Currency {
  id: number
  symbol: string
  name: string
  flag: string
  color: string
  accentClass: string
  borderClass: string
  textClass: string
  bgClass: string
}

export const CURRENCIES: Currency[] = [
  {
    id: 410,
    symbol: 'KRW',
    name: 'Korean Won',
    flag: '🇰🇷',
    color: '#3b82f6',
    accentClass: 'text-blue-400',
    borderClass: 'border-blue-500/30',
    textClass: 'text-blue-300',
    bgClass: 'bg-blue-500/5',
  },
  {
    id: 392,
    symbol: 'JPY',
    name: 'Japanese Yen',
    flag: '🇯🇵',
    color: '#ef4444',
    accentClass: 'text-red-400',
    borderClass: 'border-red-500/30',
    textClass: 'text-red-300',
    bgClass: 'bg-red-500/5',
  },
  {
    id: 978,
    symbol: 'EUR',
    name: 'Euro',
    flag: '🇪🇺',
    color: '#f59e0b',
    accentClass: 'text-amber-400',
    borderClass: 'border-amber-500/30',
    textClass: 'text-amber-300',
    bgClass: 'bg-amber-500/5',
  },
]

export function createClient(rpcUrl: string) {
  return createPublicClient({ transport: http(rpcUrl) })
}

/** raw (bigint from chain) → display string with 6 decimals */
export function formatPrice(raw: bigint): string {
  const SCALE = 1_000_000n
  const whole = raw / SCALE
  const frac = raw % SCALE
  return `${Number(whole).toLocaleString('en-US')}.${frac.toString().padStart(6, '0')}`
}

/** raw → JS number (for chart math) */
export function toFloat(raw: bigint): number {
  return Number(raw) / 1_000_000
}

export interface PriceSnapshot {
  blockNumber: bigint
  timestamp: number
  prices: Record<number, bigint>
}

const SCALE = 1_000_000n

export interface CrossPair {
  base: Currency
  quote: Currency
  /** base units per 1 quote */
  label: string
}

export const CROSS_PAIRS: CrossPair[] = [
  { base: CURRENCIES[0], quote: CURRENCIES[1], label: 'KRW / JPY' }, // KRW per 1 JPY
  { base: CURRENCIES[0], quote: CURRENCIES[2], label: 'KRW / EUR' }, // KRW per 1 EUR
  { base: CURRENCIES[1], quote: CURRENCIES[2], label: 'JPY / EUR' }, // JPY per 1 EUR
  { base: CURRENCIES[1], quote: CURRENCIES[0], label: 'JPY / KRW' }, // JPY per 1 KRW
  { base: CURRENCIES[2], quote: CURRENCIES[0], label: 'EUR / KRW' }, // EUR per 1 KRW
  { base: CURRENCIES[2], quote: CURRENCIES[1], label: 'EUR / JPY' }, // EUR per 1 JPY
]

/**
 * Derive a cross rate from two USD-base prices.
 * base/quote = price(base) / price(quote)
 * Both inputs are 6-decimal fixed-point; result is also 6-decimal.
 * Returns 0n if either price is zero.
 */
export function crossRate(baseUsd: bigint, quoteUsd: bigint): bigint {
  if (baseUsd === 0n || quoteUsd === 0n) return 0n
  return (baseUsd * SCALE) / quoteUsd
}
