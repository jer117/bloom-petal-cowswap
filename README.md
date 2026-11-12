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
| `[network]/orders/[wallet]/[index]/drafts/` | dir | The account's drafts, listed from its private store. Ids begin with their creation time, so the newest draft is listed last |
| `…/drafts/<id>/order.json` | read | The reviewable intent: quote, slippage bound, the exact order to sign, and any pending approval |
| `…/drafts/<id>/confirm` | signing write (`cowswap.order.v1`) | Sign the EIP-712 order through Bloom, which shows the owner every order field, and POST it; idempotent |
| `…/drafts/<id>/status.json` | read | Local draft state merged with live orderbook status, looked up by the order's UID |

`[network]` ∈ `mainnet`, `xdai` (Gnosis), `arbitrum_one`, `base`, `sepolia` — the
exact API environment segments under `https://api.cow.fi`. Sepolia is the
11155111 testnet deployment; the GPv2 contracts live at the same addresses as
mainnet there (verified via the settlement's live `domainSeparator()`).

## Draft lifecycle

`order.json` and `status.json` report the draft's `status`:

| Status | Meaning | What to do |
| --- | --- | --- |
| `quoted` | Created, or signed but not yet posted | Write `{"confirm":true}` to `confirm` |
| `signing_pending` | Bloom is waiting for the owner to approve the request named in `approval.action_id` | Approve it in Bloom, then write the same confirm again |
| `approval_lapsed` | The approval expired or was declined | Create a new draft |
| `ambiguous` | The order was signed, but the result of posting it was not recorded | Read `status.json`, which looks the order up, or write confirm again, which does the same before posting anything |
| `posted` | The orderbook holds the order | Follow it in `status.json` |
| `rejected` | The orderbook refused the order | Create a new draft |

A confirm that does not post the order returns an error whose message says
which of these applies. The order's UID is fixed by the order itself, so a
retried confirm can never create a second order: the orderbook refuses a
duplicate, and this Petal records the first one.

## Building

Components build from the route crate's committed lockfile, with the petal CLI
of the SDK commit that lockfile pins. `scripts/build.sh` runs that CLI through
`scripts/petal.sh`, which installs it on first use, so the build never depends
on whichever `petal` is on PATH:

```sh
cargo test --manifest-path route/Cargo.toml --locked
scripts/build.sh
scripts/petal.sh check
```

Then validate the built package with `bloom petals build .`.

`route/Cargo.toml` pins the SDK by commit until the SDK's `v0.2.0` release is
tagged. To move to another SDK commit, change `rev` there, run
`cargo update --manifest-path route/Cargo.toml -p bloom-petal-sdk`, and set the
same commit in `.github/workflows/release.yml`. Once the tag exists, pin
`tag = "v0.2.0"` instead.

To build with a petal CLI you built yourself:

```sh
PETAL_BIN=/path/to/petal scripts/build.sh
```

Before pushing, enforce the route architecture rules from the
`bloom-petal-development` skill:

```sh
bash scripts/check-route-architecture.sh
```

## Releasing

Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`, which builds and
packages with the petal CLI of the pinned SDK commit and publishes the
archive, its checksum and `petal-release.json`. The workflow takes an SDK
commit only if it is on `bloom-directory/petal`'s `master` or is a release tag.
The commit pinned today is on a pull request branch, so a release has to wait
until that work is merged and the pin moves to the merged commit.

See `DESIGN.md` for the design decisions, verification vectors, and known
gaps.
