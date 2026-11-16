# cowswap operating contract

Bloom's own `AGENTS.md` governs approvals, the outbox and policy. Routes, the
draft body and the status table are in `README.md`. Below, `<o>` is
`petals/cowswap/<net>/orders/<acct>`.

## Place an order

1. Write the draft to `<o>/new`, then list `<o>/drafts/`. The new id is the
   last one.
2. Read `<o>/drafts/<id>/order.json`. Check that `order_to_sign` is the trade
   you drafted: it is what the owner will sign.
3. Check both of these. If one fails, stop, tell the owner what is missing,
   and fix it as "Before the first order" in `README.md` says only when they
   agree:
   - the `allowance` route for the sell token shows `balance` and `allowance`
     of at least `order_to_sign.sell_amount`;
   - `wallets/<wallet>/policy.json` lists CoW's vault relayer for this chain.
4. Tell the owner the order's `sell_amount`, `buy_amount` (the least it buys)
   and `valid_to`. Write `{"confirm":true}` to `<o>/drafts/<id>/confirm`. It
   fails with `owner approval required … (request <request>)`. The owner's
   link is the `ceremony_url` in the first of these that exists, at the mount
   root. It lasts five minutes:
   - `petal-signing-requests/<request>.json`: approval of this order.
   - `wallets/<wallet>/policy-updates/pending/<request>/status.json`: on a
     wallet's first order, a policy update that allows this Petal. Tell the
     owner that is what it is.
5. After the owner approves, write the same confirm again. Success means the
   order is posted. After a policy update it asks once more, for the order.
6. Read `<o>/drafts/<id>/status.json` until `order.status`, which is CoW's, is
   no longer `open`: `fulfilled`, `cancelled` or `expired`. Report a fill from
   `order.executedSellAmount` and `order.executedBuyAmount`.

## Spend few reads

- Skip the quote routes unless the owner asked for a price. `new` takes its
  own quote.
- After confirm, `status.json` replaces `order.json` and `history.json`.
- Do not repeat `confirm` while the request awaits approval. It returns the
  same message.

## When it stops

- A failed write says what to do next. For anything else, read `status.json`
  and act on the status table.
- `outside wallet policy`: tell the owner the policy needs CoW's vault
  relayer. Change policy only if they ask.
- `outcome still unknown`: read `status.json` until it says `posted`. Do not
  draft the same trade again first.

Working on this Petal's code: `DEVELOPMENT.md`.
