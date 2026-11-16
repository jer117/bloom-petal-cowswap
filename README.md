# cowswap

Trade on CoW Protocol through Bloom. An order is posted only after the owner
approves that exact order in Bloom; the Petal holds no keys. The steps are in
`AGENTS.md`; building and releasing are in `DEVELOPMENT.md`.

## Routes

Under `petals/cowswap/`:

| Path | Purpose |
| --- | --- |
| `status.json` | Networks and contract addresses |
| `<net>/quote/sell/<sell>-<buy>/<amount>.json` | Quote for selling `<amount>` |
| `<net>/quote/buy/<sell>-<buy>/<amount>.json` | Quote for buying `<amount>` |
| `<net>/allowance/<acct>/<token>.json` | `balance`, and `allowance` to CoW's vault relayer |
| `<net>/orders/<acct>/history.json` | The account's orders on CoW |
| `<net>/orders/<acct>/new` | Write a draft body. Nothing is signed |
| `<net>/orders/<acct>/drafts/` | Draft ids, newest last |
| `…/drafts/<id>/order.json` | The exact order to be signed |
| `…/drafts/<id>/confirm` | Write `{"confirm":true}`: sign with the owner's approval, then post |
| `…/drafts/<id>/status.json` | The draft's state, and the live order once posted |

- `<net>`: `mainnet` (default), `xdai`, `arbitrum_one`, `base` or `sepolia`.
  Bloom calls those chains `ethereum`, `gnosis`, `arbitrum`, `base`, `sepolia`.
- `<acct>`: `<wallet>/<n>`.
- A token is its `0x` address. An amount is a decimal string in the token's
  smallest unit.

## Draft body

`{"sell_token":"0x…","buy_token":"0x…","kind":"sell","amount":"10000000","slippage_bps":50}`

`kind` is `sell` (`amount` is sold) or `buy` (`amount` is received).
`slippage_bps` is optional: 1 to 5000, default 50. Any other field is refused.
The quote holds for 30 minutes; after that `confirm` fails with
`quote has expired` and a new draft is needed. An order worth less than CoW's
fee is refused with `SellAmountDoesNotCoverFee`; a quote's `feeAmount` is that
fee. A native coin cannot be sold: sell its wrapped token.

## Before the first order

`<relayer>` is CoW's vault relayer,
`0xC92E8bdf79f0507f65a392b0ab4667716BFE0110` on every network. An order is
refused unless:

1. The wallet policy allows this Petal and the relayer.
   - The Petal: Bloom asks for this itself. The first `confirm` on a wallet
     returns a policy update for the owner to approve, before the order's own
     approval.
   - The relayer: `wallets/<wallet>/policy.json` needs
     `{"chain":"<chain>","destination":"<relayer>"}` in `allowed_destinations`.
     Bloom does not add it. Without it `confirm` fails with
     `outside wallet policy`. The token needs no entry.
2. The sell token's allowance to the relayer is at least the order's
   `sell_amount`. To grant it, stage
   `{"kind":"approve","token":"<sell token>","spender":"<relayer>","amount":"<sell_amount>"}`
   at `wallets/<wallet>/<n>/chains/<chain>/outbox/new.tx`. It is one on-chain
   transaction. Otherwise CoW refuses the order with `InsufficientAllowance`.
3. Bloom has the chain. If `chains/<chain>/` is missing, the `allowance` route
   fails, and the operator has to add the chain to Bloom's config. `sepolia`
   is not there by default.

## Draft status

| `status` | Meaning | Next |
| --- | --- | --- |
| `quoted` | Not yet signed | Write `confirm` |
| `signing_pending` | The owner has not approved the request in `approval.action_id` | After they approve, write the same `confirm` |
| `approval_lapsed` | The approval expired or was declined | Create a new draft; this one cannot be retried |
| `ambiguous` | Signed, but the result of posting is unknown | Read `status.json`, which looks the order up |
| `posted` | CoW holds the order; its live state is under `order` | Read `status.json` |
| `rejected` | CoW refused it; `last_error` says why | Fix the cause, then create a new draft |

The order's id is fixed by the order itself, so repeating `confirm` can never
create a second order.
