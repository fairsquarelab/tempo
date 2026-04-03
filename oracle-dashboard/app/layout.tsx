import type { Metadata } from 'next'
import './globals.css'

export const metadata: Metadata = {
  title: 'Tempo Oracle Dashboard',
  description: 'Real-time FX oracle price feed for Tempo network',
}

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className="dark">
      <body className="min-h-screen bg-zinc-950 text-zinc-100 grid-bg">
        {children}
      </body>
    </html>
  )
}
