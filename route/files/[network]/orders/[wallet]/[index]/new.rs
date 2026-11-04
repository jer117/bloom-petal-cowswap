// orders/<wallet>/<index>/new
// Write a JSON body to create a reviewable order draft:
//   {"sell_token":"0x…","buy_token":"0x…","kind":"sell","amount":"1000000","slippage_bps":50}
// `amount` is sellAmountBeforeFee (sell) or buyAmountAfterFee (buy) in atoms.
// The draft stores a live quote and the exact slippage-bounded order that
// confirm will sign. Nothing is posted at this step.
petal::route_file!(
    spec: petal::write_spec().caps(&["bloom:http", "bloom:store", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(
            br#"write JSON to create a reviewable order draft, e.g. {"sell_token":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","buy_token":"0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","kind":"sell","amount":"10000000000","slippage_bps":50}
"#.to_vec(),
        )
    },
    write: |ctx: &petal::Ctx, body: &[u8]| {
        let network = match petal::param(ctx, "network") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let network = match crate::cow::CowNetwork::from_segment(network) {
            Some(network) => network,
            None => return petal::error(-3, "unknown network; expected mainnet, xdai, arbitrum_one or base"),
        };
        let account = match petal::wallet_account(ctx) {
            Ok(account) => account,
            Err(resp) => return resp,
        };
        crate::cow::draft::create_draft(network, &account, body)
    }
);
