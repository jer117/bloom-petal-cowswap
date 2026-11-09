# cowswap

A Bloom Petal exposing CoW Protocol (CoW Swap) to AI agents through the VFS:
read-only orderbook queries (quotes, account order history, order status,
ERC20 allowances) and a reviewable order flow — quote → draft → owner-signed
EIP-712 order → orderbook submission. The Petal never holds keys; Bloom signs
after owner approval under the intent `cowswap.order.v1`.

## Route map

| Route | Kind | Purpose |
| --- | --- | --- |
| `status.json` | read | Petal identity, supported networks, contract addresses, appData info |
| `[network]/quote/sell/<0xSELL-0xBUY>/<sellAmountBeforeFee>.json` | read (5s TTL) | Live sell quote (`POST /api/v1/quote`) |
| `[network]/quote/buy/<0xSELL-0xBUY>/<buyAmountAfterFee>.json` | read (5s TTL) | Live buy quote |
| `[network]/allowance/[wallet]/[index]/<0xTOKEN>.json` | read (chain) | ERC20 allowance to the GPv2 Vault Relayer + token balance, via `eth_call` |
| `[network]/orders/[wallet]/[index]/new` | write | Create a reviewable order draft from a quote (nothing is posted) |
| `[network]/orders/[wallet]/[index]/history.json` | read | The account's live orders (`GET /api/v1/account/{owner}/orders`) |
| `[network]/orders/[wallet]/[index]/drafts/` | dir | The account's drafts, listed from its private store |
| `…/drafts/<id>/order.json` | read | The reviewable intent: quote, slippage bound, the exact order to sign |
| `…/drafts/<id>/confirm` | signing write (`cowswap.order.v1`) | Sign the EIP-712 order through Bloom, which shows the owner every order field, and POST it; idempotent |
| `…/drafts/<id>/status.json` | read | Local draft state merged with live orderbook status |

`[network]` ∈ `mainnet`, `xdai` (Gnosis), `arbitrum_one`, `base`, `sepolia` — the
exact API environment segments under `https://api.cow.fi`. Sepolia is the
11155111 testnet deployment; the GPv2 contracts live at the same addresses as
mainnet there (verified via the settlement's live `domainSeparator()`).

## Building

Components build from the route crate's tested lockfile. Run the route tests
once, which writes `route/Cargo.lock`, and commit it; then build this Petal
with `petal build` and validate the resulting package with
`bloom petals build .`:

```sh
cargo test --manifest-path route/Cargo.toml --locked
petal build --root .
```

Before pushing, enforce the route architecture rules from the
`bloom-petal-development` skill:

```sh
bash scripts/check-route-architecture.sh
petal check --root .
```

See `DESIGN.md` for the design decisions, verification vectors, and known
gaps.
