// drafts/<id>/order.json — the reviewable draft: quote, slippage bound, the
// exact order that confirm signs, and the approval requirement.
//
// Not cached (TTL 0): a confirm changes the draft's status and pending
// approval, and Bloom keeps a cached read when the confirm write reports an
// error, which is how "approval required" is reported.
petal::route_file!(spec: petal::http_read_spec(0).caps(&["bloom:store"]), read: |ctx: &petal::Ctx| {
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
    if let Err(e) = crate::cow::params::parse_draft_id(id) {
        return petal::error(-3, format!("draft id: {e}"));
    }
    let key = crate::cow::draft::draft_key(
        network.api_env(),
        account.wallet(),
        account.number(),
        id,
    );
    match crate::cow::draft::load_draft(&key) {
        Ok(draft) => petal::read_json_value(&crate::cow::draft::project_draft(&draft)),
        Err(resp) => resp,
    }
});
