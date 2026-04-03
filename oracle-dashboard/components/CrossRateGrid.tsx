'use client'

import { useEffect, useRef, useState } from 'react'
import { CROSS_PAIRS, CrossPair, crossRate, formatPrice, toFloat } from '@/lib/oracle'
import { Sparkline } from './Sparkline'

interface CrossRateGridProps {
  /** USD-base prices indexed by currency id */
  latest: Record<number, bigint>
  history: Record<number, bigint[]>
}

export function CrossRateGrid({ latest, history }: CrossRateGridProps) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-300">Cross Rates</h2>
        <span className="text-xs text-zinc-600">derived · base / quote</span>
      </div>
      <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
        {CROSS_PAIRS.map((pair) => (
          <CrossRateCard
            key={pair.label}
            pair={pair}
            current={crossRate(latest[pair.base.id] ?? 0n, latest[pair.quote.id] ?? 0n)}
            sparkData={(history[pair.base.id] ?? []).map((b, i) =>
              toFloat(crossRate(b, history[pair.quote.id]?.[i] ?? 0n))
            )}
          />
        ))}
      </div>
    </div>
  )
}

function CrossRateCard({
  pair,
  current,
  sparkData,
}: {
  pair: CrossPair
  current: bigint
  sparkData: number[]
}) {
  const prevRef = useRef<bigint | null>(null)
  const [flash, setFlash] = useState<'up' | 'down' | null>(null)

  useEffect(() => {
    const prev = prevRef.current
    if (prev === null || current === prev) return
    setFlash(current > prev ? 'up' : 'down')
    const t = setTimeout(() => setFlash(null), 700)
    return () => clearTimeout(t)
  }, [current])

  useEffect(() => { prevRef.current = current }, [current])

  const color = pair.base.color

  return (
    <div
      className={`
        rounded-xl border p-4 flex flex-col gap-2 overflow-hidden
        transition-all duration-300
        ${flash === 'up' ? 'ring-1 ring-green-500/40' : ''}
        ${flash === 'down' ? 'ring-1 ring-red-500/40' : ''}
      `}
      style={{ borderColor: `${color}30`, background: `${color}08` }}
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-1.5 text-xs font-medium" style={{ color }}>
          <span>{pair.base.flag}</span>
          <span className="text-zinc-600">/</span>
          <span>{pair.quote.flag}</span>
          <span className="ml-1">{pair.label}</span>
        </div>
      </div>

      {/* Price */}
      <div
        className={`
          font-mono text-lg font-bold tracking-tight text-white
          transition-colors duration-200
          ${flash === 'up' ? 'text-green-300' : ''}
          ${flash === 'down' ? 'text-red-300' : ''}
        `}
      >
        {current === 0n ? '—' : formatPrice(current)}
      </div>

      {/* Sparkline */}
      <div className="-mx-1 mt-0.5">
        <Sparkline data={sparkData.filter(v => v > 0)} color={color} height={32} />
      </div>
    </div>
  )
}
