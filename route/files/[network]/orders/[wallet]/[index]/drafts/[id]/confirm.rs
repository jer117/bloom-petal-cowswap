// drafts/<id>/confirm
// Write {"confirm":true} to sign the draft's EIP-712 GPv2 order with this
// account's key through the Bloom signing host and POST it to the orderbook.
// Signing runs under the intent "cowswap.order.v1"; the first attempt
// typically returns -3 "owner approval required" — approve it in Bloom, then
// retry this exact write. Signature bytes are never stored.
petal::route_file!(
    spec: petal::signing_write_spec(crate::cow::draft::SIGNING_INTENT)
        .caps(&["bloom:http", "bloom:store", "bloom:sign", "bloom:vfs.read"]),
    read: |_ctx: &petal::Ctx| {
        petal::DispatchResponse::Read(
            br#"write {"confirm":true} to sign and post this draft's order to the CoW orderbook. This is a value-moving order submission.
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
            None => return petal::error(-3, "unknown network; expected mainnet, xdai, arbitrum_one, base or sepolia"),
        };
        let account = match petal::wallet_account(ctx) {
            Ok(account) => account,
            Err(resp) => return resp,
        };
        let id = match petal::param(ctx, "id") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        crate::cow::confirm::confirm_draft(ctx, network, &account, id, body)
    }
);
