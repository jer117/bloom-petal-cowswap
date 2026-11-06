# CoW Swap Petal — Design

A Bloom Petal exposing CoW Protocol to agents: orderbook reads, ERC20
allowance checks, and a reviewable order draft → sign → post flow. Bloom owns
keys and approvals; the Petal builds the EIP-712 order and asks the host to
sign under a declared intent.

## Route tree

```
$index.rs                                  → status.json + the five networks
status.json.rs                             → identity, contracts, appData
[network]/                                 → mainnet | xdai | arbitrum_one | base | sepolia
  $index.rs                                → quote, allowance, orders
  quote/
    $index.rs                              → sell, buy
    sell|buy/$index.rs                     → (dynamic [pair])
    sell|buy/[pair]/$index.rs              → (dynamic [amount])
    sell|buy/[pair]/[amount].json.rs       → POST /api/v1/quote (http read, 5s)
  allowance/
    $index.rs, [wallet]/$index.rs          → Bloom-supplied inventory
    [wallet]/[index]/$index.rs             → (dynamic [token])
    [wallet]/[index]/[token].json.rs       → eth_call allowance/balanceOf
  orders/
    $index.rs, [wallet]/$index.rs          → Bloom-supplied inventory
    [wallet]/[index]/$index.rs             → new, history.json, drafts/
    [wallet]/[index]/new.rs                → WRITE: quote + slippage → draft
    [wallet]/[index]/history.json.rs       → GET /account/{owner}/orders
    [wallet]/[index]/drafts/$index.rs      → draft ids from the account store
    [wallet]/[index]/drafts/[id]/
      $index.rs                            → order.json, confirm, status.json
      order.json.rs                        → reviewable intent (store read)
      confirm.rs                           → SIGNING WRITE (cowswap.order.v1)
      status.json.rs                       → local state + live order status
```

`[network]` values are the exact API environment segments (`xdai` is Gnosis,
`arbitrum_one` is Arbitrum One), validated against a closed enum in
`cow::network` before any URL or chain name is built. `[wallet]/[index]` are
Bloom-resolved; account addresses come from the wallet projection, never from
a path segment.

## Capabilities and policy

- Package ceiling (`petal.toml [caps].allowed`): `bloom:http`,
  `bloom:store`, `bloom:sign`, `bloom:chain`, `bloom:vfs.read`. No
  `bloom:tx.outbox` (orders are off-chain intents), no `bloom:vfs.write`.
- Per-route specs are the narrowest available: `static_dir_spec` /
  `static_read_spec` for listings and static reads, `http_read_spec(5_000)`
  for quotes, `wallet_http_read_spec` for history, `account_read_spec` for
  draft status, `store_read_spec`/`store_dir_spec` for draft files, and
  `.caps(&[...])` overrides where the default write/chain specs would
  over-grant (`new`: http+store+vfs.read; `confirm`: http+store+sign+vfs.read;
  allowance: chain+vfs.read).
- Network egress: one `[[net.allow]]` for GET across the five environments'
  `/api/v1/*` trees, and one for POST pinned to exactly `/quote` and
  `/orders` in each environment. No other host, method, or path.
- Signing: `[sign].allowed_intents = ["cowswap.order.v1"]` only.
- Store: namespace `state` only; keys `orders/<env>/<wallet>/<account>/<id>/…`.
  No secret namespace is used — nothing secret is ever stored.

## Order flow

1. **new** — validate body against strict grammars (`cow::params`: address,
   decimal atoms, side enum, slippage 1–5000 bps default 50), resolve the
   account address, `POST /api/v1/quote` (`priceQuality: verified`,
   `validFor: 300`, this Petal's appData document), bound with slippage:
   sell orders set `buyAmount = floor(quoted × (1−bps))`, buy orders set
   `sellAmount = ceil(quoted × (1+bps))`. Store the draft (quote + the exact
   order to sign) under a fresh `od-<hex16>` id.
2. **confirm** — reload the draft; idempotent if already posted; an ambiguous
   prior POST is reconciled first by looking up the locally computed order UID
   (`digest ‖ owner ‖ validTo`). Refuse expired quotes (validTo − 60s), then:
   build the EIP-712 payload, declare a claim (`moves_value` with a debit of
   sellAmount+feeAmount in the sell token, destination = GPv2VaultRelayer; no
   declared network fee — Bloom currently refuses those and CoW fees live
   inside the signed amounts), request the signature
   (`sign_payload`, Exact selector, deterministic `action` bytes so retries
   match the owner's approval), verify the signature recovers the draft
   owner, then `POST /api/v1/orders`. The API's returned UID must be the
   locally computed one. Signature bytes are never persisted.
3. **status** — local draft state, merged with `GET /api/v1/orders/{uid}`
   once posted.

The signing preimage is `0x1901 ‖ domainSeparator ‖ hashStruct(Order)` with
domain `{name:"Gnosis Protocol", version:"v2", chainId, verifyingContract:
GPv2Settlement}`. The sol! struct is named `Order` deliberately — alloy
derives the typehash from the struct name, and the settlement contract hashes
`Order(…)`. (The struct was originally named `Gpv2Order`, which produced
wrong digests; the independent cross-check below caught it.)

## appData

`{"appCode":"bloom-cowswap-petal","environment":"production","metadata":{},"version":"1.16.0"}`
serialized with sorted keys (deterministic bytes), sent as the full JSON
string with every order; the signed order's `appData` field is
`keccak256(utf8(document))` =
`0xb59ec5fad15447b301bafeef096c7874d24b4a80611f9ecefd532b0ecccf4326`.
The `appDataHash` field is sent alongside for debugging, per the OpenAPI.

## Verification (all pinned in unit tests)

- **Live API shapes**: quote (`POST /api/v1/quote`, USDC→WETH sell on
  mainnet, 2026-11-04) and account orders (`GET
  /api/v1/account/0x00…01/orders`) responses were captured and pinned in
  `cow::quote` / `cow::account_orders` tests; projections are derived from
  them.
- **Order encoding**: cross-checked three independent ways:
  1. a pure-Python EIP-712 implementation (spec-following, keccak via a
     public JSON-RPC node's `web3_sha3`) — digest
     `0x5a517609d176fc19edf41688e647d167c456e3f461ea5caca43057153857ac20`
     for the fixed mainnet test order, and three more chain-bound vectors;
  2. the **live** `GPv2Settlement.domainSeparator()` on mainnet
     (`0xc078f884a2676e1345748b1feace7b0abee5d00ecadb6e574dcdd109a63e8943`)
     equals the domain separator this Petal derives;
  3. the canonical `GPv2Order.TYPE_HASH`
     (`0xd5a25ba2e97094ad7d83dc28a6572da797d6b3e7fc6663bd93efb789fc17e489`)
     from `cowprotocol/contracts` matches what alloy derives from the struct.
  The keccak source itself is validated against the docs' published vectors
  (`keccak256("cow")`, `keccak256("{}")`).
- **Sepolia (chain 11155111)**: added 2026-11-04. GPv2Settlement and
  GPv2VaultRelayer deploy at the same addresses as mainnet; the live
  `GPv2Settlement.domainSeparator()` eth_call on Sepolia
  (`0xdaee378bd0eb30ddf479272accf91761e697bc00e067a268f95f1d2732ed230b`,
  via ethereum-sepolia-rpc.publicnode.com) equals the domain separator this
  Petal derives for `CowNetwork::Sepolia`, and a fourth chain-bound digest
  vector is pinned in `cow::order` tests. The end-to-end exercise lives in
  `route/examples/sepolia_e2e.rs` (dev-only; never part of the wasm Petal).
- **Signature path**: an end-to-end test builds the order, hashes it, signs
  the digest with a known local key (anvil account 0), and asserts the
  signature recovers the expected address and round-trips through the
  65-byte `r‖s‖v` (v∈{27,28}) wire form the API expects.

## Gaps and deliberate non-features

- **Token metadata endpoint is gone.** `GET /api/v1/token/{token}` returns
  404 on the live API (mainnet and xdai, 2026-11-04) and is absent from the
  current orderbook OpenAPI. The planned `tokens/[token].json` route was
  dropped rather than built on a dead endpoint; amounts are quoted and
  carried in atoms throughout.
- **No REST allowance endpoint.** `GET /api/v1/allowance/*` returns 404 and
  is not in the OpenAPI. Allowances are read from chain via `eth_call`
  (`bloom:chain` cap) against the GPv2VaultRelayer instead — the dedicated
  allowance route also verifies Bloom's chain route matches the expected
  chain id before signing flows rely on it.
- **Order cancellation is out of scope** (stretch goal skipped): it needs an
  `OrderCancellations(bytes[])` EIP-712 signature plus `DELETE
  /api/v1/orders`, which the net policy deliberately does not allow.
- **appData registration is never exercised in development** (`POST
  /api/v1/app_data` is a write endpoint); orders carry the full appData
  string so registration is unnecessary anyway.
- **No x402/paid quotes, no API keys** — unauthenticated endpoints only, per
  the package constraints.
- The broker-side review of `cowswap.order.v1` claims depends on Bloom's
  operation-class catalog and installer provenance; this Petal declares
  honest debits/destinations, but nothing here can prove how a given Broker
  renders them.
