'use client'

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  CURRENCIES,
  ORACLE_ABI,
  ORACLE_ADDRESS,
  PriceSnapshot,
  createClient,
} from '@/lib/oracle'
import { PriceCard } from '@/components/PriceCard'
import { HistoryTable } from '@/components/HistoryTable'
import { CrossRateGrid } from '@/components/CrossRateGrid'
import { CreatePoolModal } from '@/components/CreatePoolModal'

// Relative path (/api/rpc) works in browser; make it absolute with window.location.origin
const RPC_URL_ENV = process.env.NEXT_PUBLIC_RPC_URL ?? '/api/rpc'
const getRpcUrl = () =>
  typeof window !== 'undefined' && RPC_URL_ENV.startsWith('/')
    ? `${window.location.origin}${RPC_URL_ENV}`
    : RPC_URL_ENV
const HISTORY_SIZE = 30
const POLL_MS = 400

type Status = 'connecting' | 'live' | 'error'

export default function Home() {
  const [status, setStatus] = useState<Status>('connecting')
  const [blockNumber, setBlockNumber] = useState<bigint | null>(null)
  const [snapshots, setSnapshots] = useState<PriceSnapshot[]>([])
  const [lastPoll, setLastPoll] = useState<Date | null>(null)
  const [showCreatePool, setShowCreatePool] = useState(false)
  const lastBlockRef = useRef<bigint | null>(null)

  const fetchPrices = useCallback(async () => {
    const rpcUrl = getRpcUrl()
    const client = createClient(rpcUrl)
    try {
      const [block, ...rawPrices] = await Promise.all([
        client.getBlockNumber(),
        ...CURRENCIES.map((c) =>
          client.readContract({
            address: ORACLE_ADDRESS,
            abi: ORACLE_ABI,
            functionName: 'getOraclePrice',
            args: [c.id],
          })
        ),
      ])

      console.log('[oracle] block', block, 'prices', rawPrices)

      // Skip if same block
      if (lastBlockRef.current === block) return
      lastBlockRef.current = block

      const prices: Record<number, bigint> = {}
      CURRENCIES.forEach((c, i) => {
        prices[c.id] = rawPrices[i] as bigint
      })

      const snap: PriceSnapshot = {
        blockNumber: block,
        timestamp: Date.now(),
        prices,
      }

      setBlockNumber(block)
      setSnapshots((prev) => [snap, ...prev].slice(0, HISTORY_SIZE))
      setLastPoll(new Date())
      setStatus('live')
    } catch (e) {
      console.error('[oracle] fetch error', e)
      setStatus('error')
    }
  }, [])

  useEffect(() => {
    fetchPrices()
    const id = setInterval(fetchPrices, POLL_MS)
    return () => clearInterval(id)
  }, [fetchPrices])

  // Build per-currency history (oldest → newest) for sparklines
  const currencyHistory = (currencyId: number): bigint[] =>
    [...snapshots].reverse().map((s) => s.prices[currencyId] ?? 0n)

  // Map of currencyId → history array (for cross rate sparklines)
  const allHistory: Record<number, bigint[]> = Object.fromEntries(
    CURRENCIES.map((c) => [c.id, currencyHistory(c.id)])
  )

  const latest = snapshots[0]
  const latestPrices: Record<number, bigint> = latest?.prices ?? {}

  return (
    <div className="min-h-screen flex flex-col">
      {/* Header */}
      <header className="border-b border-zinc-800/60 backdrop-blur-sm sticky top-0 z-10 bg-zinc-950/80">
        <div className="max-w-6xl mx-auto px-6 h-16 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-7 h-7 rounded-lg bg-gradient-to-br from-blue-500 to-purple-600 flex items-center justify-center text-sm font-bold">
              T
            </div>
            <div>
              <span className="font-semibold text-white">Tempo</span>
              <span className="text-zinc-400 ml-1.5 text-sm">Oracle Dashboard</span>
            </div>
          </div>

          <div className="flex items-center gap-4">
            {/* Create Pool */}
            <button
              onClick={() => setShowCreatePool(true)}
              className="text-xs font-medium px-3 py-1.5 rounded-lg bg-blue-500/10 text-blue-400 border border-blue-500/20 hover:bg-blue-500/20 hover:border-blue-500/40 transition-all"
            >
              + Create Pool
            </button>

            {/* RPC */}
            <div className="hidden sm:flex items-center gap-1.5 text-xs font-mono text-zinc-500 bg-zinc-900 rounded-lg px-3 py-1.5 border border-zinc-800">
              {RPC_URL_ENV}
            </div>

            {/* Block number */}
            {blockNumber !== null && (
              <div className="flex items-center gap-1.5 text-xs font-mono bg-zinc-900 rounded-lg px-3 py-1.5 border border-zinc-800 text-zinc-300">
                <span className="text-zinc-500">#</span>
                {blockNumber.toLocaleString()}
              </div>
            )}

            {/* Status */}
            <div className="flex items-center gap-2">
              <span
                className={`
                  w-2 h-2 rounded-full
                  ${status === 'live' ? 'bg-green-500 shadow-[0_0_6px_rgba(34,197,94,0.8)]' : ''}
                  ${status === 'connecting' ? 'bg-amber-500 animate-pulse' : ''}
                  ${status === 'error' ? 'bg-red-500' : ''}
                `}
              />
              <span className="text-xs text-zinc-400">
                {status === 'live' ? 'Live' : status === 'connecting' ? 'Connecting...' : 'Error'}
              </span>
            </div>
          </div>
        </div>
      </header>

      <main className="flex-1 max-w-6xl mx-auto w-full px-6 py-8 flex flex-col gap-8">
        {/* Meta row */}
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-xl font-semibold text-white">FX Oracle Prices</h1>
            <p className="text-sm text-zinc-500 mt-0.5">
              On-chain price feed · 6-decimal fixed-point · units per 1 USD
            </p>
          </div>
          {lastPoll && (
            <div className="text-xs text-zinc-600 font-mono">
              Updated {lastPoll.toLocaleTimeString()}
            </div>
          )}
        </div>

        {/* Price cards */}
        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
          {CURRENCIES.map((currency) => (
            <PriceCard
              key={currency.id}
              currency={currency}
              current={latest?.prices[currency.id] ?? 0n}
              history={currencyHistory(currency.id)}
            />
          ))}
        </div>

        {/* Stats bar */}
        {snapshots.length >= 2 && (
          <div className="grid grid-cols-3 gap-4">
            {CURRENCIES.map((c) => {
              const vals = snapshots.slice(0, 10).map((s) => s.prices[c.id] ?? 0n)
              const floats = vals.map((v) => Number(v) / 1_000_000).filter((v) => v > 0)
              if (floats.length === 0) return null
              const hi = Math.max(...floats)
              const lo = Math.min(...floats)
              const avg = floats.reduce((a, b) => a + b, 0) / floats.length
              return (
                <div
                  key={c.id}
                  className={`rounded-xl border ${c.borderClass} p-4 flex flex-col gap-2`}
                >
                  <div className={`text-xs font-medium ${c.accentClass}`}>
                    {c.flag} {c.symbol} · last 10 blocks
                  </div>
                  <div className="grid grid-cols-3 gap-2 text-xs font-mono">
                    <div className="flex flex-col gap-0.5">
                      <span className="text-zinc-600">High</span>
                      <span className="text-green-400">{hi.toFixed(4)}</span>
                    </div>
                    <div className="flex flex-col gap-0.5">
                      <span className="text-zinc-600">Low</span>
                      <span className="text-red-400">{lo.toFixed(4)}</span>
                    </div>
                    <div className="flex flex-col gap-0.5">
                      <span className="text-zinc-600">Avg</span>
                      <span className="text-zinc-300">{avg.toFixed(4)}</span>
                    </div>
                  </div>
                </div>
              )
            })}
          </div>
        )}

        {/* Cross Rates */}
        {snapshots.length >= 1 && (
          <CrossRateGrid latest={latestPrices} history={allHistory} />
        )}

        {/* History */}
        <HistoryTable snapshots={snapshots} />
      </main>

      {/* Create Pool Modal */}
      <CreatePoolModal
        open={showCreatePool}
        onClose={() => setShowCreatePool(false)}
        rpcUrl={getRpcUrl()}
        latestPrices={latestPrices}
      />

      {/* Footer */}
      <footer className="border-t border-zinc-800/40 py-4">
        <div className="max-w-6xl mx-auto px-6 flex items-center justify-between text-xs text-zinc-600">
          <span>Tempo · Oracle Consensus</span>
          <span className="font-mono">{ORACLE_ADDRESS}</span>
        </div>
      </footer>
    </div>
  )
}
