Oracle top-of-block verification criterion (Tempo localnet)

A) Default / matches current implementation:
   When a block contains at least one transaction to TempoOracle (0x0ACC010000000000000000000000000000000000),
   the leading "oracle prefix" must be a contiguous run at the start of the block body:
   - zero or more updatePriceFeed (same selector repeated),
   - then exactly one setPriceFeed as the LAST tx in that prefix.
   Blocks with NO oracle txs are valid (empty bundle); do not treat them as failures.

B) Strict "every block must have oracle" mode:
   Not enforced by the node today. Use verify-oracle-blocks --require-every-block only if you
   intentionally want CI to fail when any block lacks an oracle prefix (may flake if the
   bundle is empty).

Quick start (consensus localnet with HTTP oracle):
  nu tempo.nu localnet --mode consensus --nodes 3 --oracle-localnet --reset

After blocks are produced (node 0 RPC on 8545):
  cargo run -p tempo-xtask -- verify-oracle-blocks --rpc http://127.0.0.1:8545 --from-block 1 --to-block 20
  # omit --to-block to scan through latest

Or: scripts/verify-oracle-top-block.sh [from] [to]

Strict mode (fail if any block lacks oracle txs):
  cargo run -p tempo-xtask -- verify-oracle-blocks --rpc ... --from-block 1 --require-every-block
