import { NextRequest, NextResponse } from 'next/server'

const RPC_URL = process.env.RPC_URL ?? 'http://localhost:8545'

export async function POST(req: NextRequest) {
  const body = await req.text()
  const response = await fetch(RPC_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body,
  })
  const data = await response.text()
  return new NextResponse(data, {
    status: response.status,
    headers: { 'Content-Type': 'application/json' },
  })
}
