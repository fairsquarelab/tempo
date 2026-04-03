'use client'

import { useEffect, useState } from 'react'
import { CURRENCIES, ORACLE_ABI, ORACLE_ADDRESS, crossRate, formatPrice } from '@/lib/oracle'
import { createPublicClient, http } from 'viem'

interface Pair {
  base: { id: number; symbol: string; flag: string }
  quote: { id: number; symbol: string; flag: string }
  price: bigint
}

interface CreatePoolModalProps {
  open: boolean
  onClose: () => void
  rpcUrl: string
  latestPrices: Record<number, bigint>
}

export function CreatePoolModal({ open, onClose, rpcUrl, latestPrices }: CreatePoolModalProps) {
  const [pairs, setPairs] = useState<Pair[]>([])
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (!open) return
    setLoading(true)

    const client = createPublicClient({ transport: http(rpcUrl) })

    client
      .readContract({
        address: ORACLE_ADDRESS,
        abi: ORACLE_ABI,
        functionName: 'getCurrencies',
      })
      .then((ids) => {
        const registered = (ids as number[]).map((id) => {
          const known = CURRENCIES.find((c) => c.id === id)
          return known ?? { id, symbol: `#${id}`, flag: '🔵', color: '#888', accentClass: '', borderClass: '', textClass: '', bgClass: '' }
        })

        // All C(n,2) combinations
        const generated: Pair[] = []
        for (let i = 0; i < registered.length; i++) {
          for (let j = i + 1; j < registered.length; j++) {
            const base = registered[i]
            const quote = registered[j]
            generated.push({
              base,
              quote,
              price: crossRate(latestPrices[base.id] ?? 0n, latestPrices[quote.id] ?? 0n),
            })
          }
        }
        setPairs(generated)
      })
      .catch(console.error)
      .finally(() => setLoading(false))
  }, [open, rpcUrl, latestPrices])

  if (!open) return null

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 bg-black/60 backdrop-blur-sm z-40"
        onClick={onClose}
      />

      {/* Modal */}
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        <div className="bg-zinc-900 border border-zinc-700 rounded-2xl w-full max-w-lg shadow-2xl">
          {/* Header */}
          <div className="flex items-center justify-between px-6 py-5 border-b border-zinc-800">
            <div>
              <h2 className="text-base font-semibold text-white">Create Pool</h2>
              <p className="text-xs text-zinc-500 mt-0.5">
                Available pairs from registered oracle currencies
              </p>
            </div>
            <button
              onClick={onClose}
              className="text-zinc-500 hover:text-zinc-300 transition-colors text-xl leading-none"
            >
              ×
            </button>
          </div>

          {/* Body */}
          <div className="px-6 py-4 flex flex-col gap-3 max-h-[60vh] overflow-y-auto">
            {loading && (
              <div className="text-center py-8 text-zinc-500 text-sm">Loading pairs...</div>
            )}

            {!loading && pairs.length === 0 && (
              <div className="text-center py-8 text-zinc-500 text-sm">
                No registered currencies found.
              </div>
            )}

            {!loading && pairs.map((pair) => (
              <div
                key={`${pair.base.id}-${pair.quote.id}`}
                className="flex items-center justify-between bg-zinc-800/50 hover:bg-zinc-800 border border-zinc-700/50 rounded-xl px-4 py-3 transition-colors group"
              >
                {/* Pair label */}
                <div className="flex items-center gap-3">
                  <div className="flex items-center -space-x-1">
                    <span className="text-xl">{pair.base.flag}</span>
                    <span className="text-xl">{pair.quote.flag}</span>
                  </div>
                  <div>
                    <div className="text-sm font-semibold text-white">
                      {pair.base.symbol} / {pair.quote.symbol}
                    </div>
                    <div className="text-xs text-zinc-500 font-mono">
                      {pair.price > 0n
                        ? `1 ${pair.quote.symbol} = ${formatPrice(pair.price)} ${pair.base.symbol}`
                        : 'Price unavailable'}
                    </div>
                  </div>
                </div>

                {/* Create button */}
                <button
                  className="text-xs font-medium px-3 py-1.5 rounded-lg bg-blue-500/10 text-blue-400 border border-blue-500/20 hover:bg-blue-500/20 hover:border-blue-500/40 transition-all opacity-0 group-hover:opacity-100"
                  onClick={() => {
                    // TODO: send createPool tx
                    alert(`Create pool: ${pair.base.symbol}/${pair.quote.symbol}`)
                  }}
                >
                  Create
                </button>
              </div>
            ))}
          </div>

          {/* Footer */}
          <div className="px-6 py-4 border-t border-zinc-800">
            <p className="text-xs text-zinc-600">
              Pool creation will be available once the AMM contract is deployed.
            </p>
          </div>
        </div>
      </div>
    </>
  )
}
