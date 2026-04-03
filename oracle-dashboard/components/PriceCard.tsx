'use client'

import { useEffect, useRef, useState } from 'react'
import { Currency, formatPrice, toFloat } from '@/lib/oracle'
import { Sparkline } from './Sparkline'

interface PriceCardProps {
  currency: Currency
  current: bigint
  history: bigint[]
}

export function PriceCard({ currency, current, history }: PriceCardProps) {
  const prevRef = useRef<bigint | null>(null)
  const [flash, setFlash] = useState<'up' | 'down' | null>(null)

  const prev = prevRef.current

  useEffect(() => {
    if (prev === null || current === prev) return
    setFlash(current > prev ? 'up' : 'down')
    const t = setTimeout(() => setFlash(null), 700)
    return () => clearTimeout(t)
  }, [current, prev])

  useEffect(() => {
    prevRef.current = current
  }, [current])

  const changePercent =
    prev && prev !== 0n
      ? ((toFloat(current) - toFloat(prev)) / toFloat(prev)) * 100
      : null

  const sparkData = history.map(toFloat)

  return (
    <div
      className={`
        relative rounded-2xl border ${currency.borderClass} ${currency.bgClass}
        p-6 flex flex-col gap-3 overflow-hidden transition-all duration-300
        ${flash === 'up' ? 'ring-1 ring-green-500/50' : ''}
        ${flash === 'down' ? 'ring-1 ring-red-500/50' : ''}
      `}
      style={{
        background: flash === 'up'
          ? `linear-gradient(135deg, rgba(34,197,94,0.08) 0%, transparent 60%)`
          : flash === 'down'
          ? `linear-gradient(135deg, rgba(239,68,68,0.08) 0%, transparent 60%)`
          : undefined,
        transition: 'background 0.6s ease-out',
      }}
    >
      {/* Header */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-2xl">{currency.flag}</span>
          <div>
            <div className={`text-sm font-semibold ${currency.accentClass}`}>{currency.symbol}</div>
            <div className="text-xs text-zinc-500">{currency.name}</div>
          </div>
        </div>
        {changePercent !== null && (
          <div
            className={`
              text-xs font-mono px-2 py-1 rounded-full
              ${changePercent >= 0
                ? 'text-green-400 bg-green-500/10'
                : 'text-red-400 bg-red-500/10'}
            `}
          >
            {changePercent >= 0 ? '▲' : '▼'} {Math.abs(changePercent).toFixed(4)}%
          </div>
        )}
      </div>

      {/* Price */}
      <div className="flex flex-col gap-0.5">
        <div
          className={`
            font-mono text-3xl font-bold tracking-tight text-white
            transition-all duration-200
            ${flash === 'up' ? 'text-green-300' : ''}
            ${flash === 'down' ? 'text-red-300' : ''}
          `}
        >
          {current === 0n ? '—' : formatPrice(current)}
        </div>
        <div className="text-xs text-zinc-500 font-mono">units per 1 USD</div>
      </div>

      {/* Sparkline */}
      <div className="mt-1 -mx-1">
        <Sparkline data={sparkData} color={currency.color} height={48} />
      </div>
    </div>
  )
}
