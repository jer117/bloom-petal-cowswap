// orders/<wallet>/<index>/history.json
// The account's live orders on this network's orderbook.
petal::route_file!(spec: petal::wallet_http_read_spec(10_000), read: |ctx: &petal::Ctx| {
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
    let owner_text = match account.address_evm() {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let owner = match owner_text.parse::<alloy::primitives::Address>() {
        Ok(address) => address,
        Err(_) => return petal::error(-4, "account EVM address is not parseable"),
    };
    match crate::cow::account_orders::account_orders(network, owner) {
        Ok(view) => petal::read_json_value(&view),
        Err(resp) => resp,
    }
});
