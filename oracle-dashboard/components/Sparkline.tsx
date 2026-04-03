'use client'

interface SparklineProps {
  data: number[]
  color: string
  height?: number
}

export function Sparkline({ data, color, height = 48 }: SparklineProps) {
  if (data.length < 2) {
    return <div style={{ height }} className="w-full" />
  }

  const width = 280
  const padding = 4
  const min = Math.min(...data)
  const max = Math.max(...data)
  const range = max - min || 1

  const points = data.map((v, i) => {
    const x = padding + (i / (data.length - 1)) * (width - padding * 2)
    const y = padding + ((1 - (v - min) / range) * (height - padding * 2))
    return [x, y] as [number, number]
  })

  // Smooth path using cubic bezier
  const d = points.reduce((acc, [x, y], i) => {
    if (i === 0) return `M ${x},${y}`
    const [px, py] = points[i - 1]
    const cx1 = px + (x - px) / 2
    const cy1 = py
    const cx2 = px + (x - px) / 2
    const cy2 = y
    return `${acc} C ${cx1},${cy1} ${cx2},${cy2} ${x},${y}`
  }, '')

  // Fill path
  const lastX = points[points.length - 1][0]
  const firstX = points[0][0]
  const fillD = `${d} L ${lastX},${height} L ${firstX},${height} Z`

  const gradientId = `spark-${color.replace('#', '')}`

  return (
    <svg width="100%" viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" style={{ height }}>
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.3" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <path d={fillD} fill={`url(#${gradientId})`} />
      <path d={d} fill="none" stroke={color} strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  )
}
