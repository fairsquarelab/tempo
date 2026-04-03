'use client'

import { CURRENCIES, formatPrice, PriceSnapshot } from '@/lib/oracle'

interface HistoryTableProps {
  snapshots: PriceSnapshot[]
}

export function HistoryTable({ snapshots }: HistoryTableProps) {
  if (snapshots.length === 0) return null

  return (
    <div className="rounded-2xl border border-zinc-800 overflow-hidden">
      <div className="px-6 py-4 border-b border-zinc-800 flex items-center justify-between">
        <h2 className="text-sm font-semibold text-zinc-300">Price History</h2>
        <span className="text-xs text-zinc-600">{snapshots.length} blocks</span>
      </div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-zinc-800/50">
              <th className="px-6 py-3 text-left text-xs font-medium text-zinc-500 uppercase tracking-wider">
                Block
              </th>
              {CURRENCIES.map((c) => (
                <th
                  key={c.id}
                  className="px-6 py-3 text-right text-xs font-medium text-zinc-500 uppercase tracking-wider"
                >
                  {c.flag} {c.symbol}
                </th>
              ))}
              <th className="px-6 py-3 text-right text-xs font-medium text-zinc-500 uppercase tracking-wider">
                Age
              </th>
            </tr>
          </thead>
          <tbody>
            {snapshots.map((snap, idx) => (
              <tr
                key={snap.blockNumber.toString()}
                className={`
                  border-b border-zinc-800/30 transition-colors
                  ${idx === 0 ? 'bg-zinc-800/20' : 'hover:bg-zinc-800/10'}
                `}
              >
                <td className="px-6 py-3 font-mono text-zinc-300">
                  #{snap.blockNumber.toLocaleString()}
                </td>
                {CURRENCIES.map((c) => {
                  const val = snap.prices[c.id]
                  const prev = snapshots[idx + 1]?.prices[c.id]
                  const up = prev && val && val > prev
                  const down = prev && val && val < prev
                  return (
                    <td
                      key={c.id}
                      className={`
                        px-6 py-3 font-mono text-right
                        ${up ? 'text-green-400' : down ? 'text-red-400' : 'text-zinc-300'}
                      `}
                    >
                      {val ? formatPrice(val) : '—'}
                    </td>
                  )
                })}
                <td className="px-6 py-3 text-right text-zinc-500 text-xs">
                  {formatAge(snap.timestamp)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function formatAge(ts: number): string {
  const diff = Math.floor((Date.now() - ts) / 1000)
  if (diff < 60) return `${diff}s ago`
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`
  return `${Math.floor(diff / 3600)}h ago`
}
