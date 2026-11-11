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

Components build from the route crate's committed lockfile, with the petal CLI
of the SDK release that lockfile pins. `scripts/build.sh` runs that CLI through
`scripts/petal.sh`, which installs it on first use, so the build never depends
on whichever `petal` is on PATH:

```sh
cargo test --manifest-path route/Cargo.toml --locked
scripts/build.sh
scripts/petal.sh check
```

Then validate the built package with `bloom petals build .`.

Until the SDK's `v0.2.0` tag exists, `route/Cargo.toml` pins the SDK by commit,
and `scripts/petal.sh` installs a CLI only for a release tag. Until then, build
with a 0.2.0 petal CLI you built yourself:

```sh
PETAL_BIN=/path/to/petal scripts/build.sh
```

Once the tag exists, change the pin to `tag = "v0.2.0"` and run
`cargo update --manifest-path route/Cargo.toml -p bloom-petal-sdk`.

Before pushing, enforce the route architecture rules from the
`bloom-petal-development` skill:

```sh
bash scripts/check-route-architecture.sh
```

## Releasing

Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`, which builds and
packages with the petal CLI of the pinned SDK release and publishes the
archive, its checksum and `petal-release.json`. It needs the SDK pinned to a
release tag, so it cannot run before the pin above changes.

See `DESIGN.md` for the design decisions, verification vectors, and known
gaps.
