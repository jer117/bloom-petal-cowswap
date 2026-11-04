// quote/sell/<0xSELLTOKEN-0xBUYTOKEN>/<sellAmountBeforeFee>.json
// A live fee-and-price quote for a sell order of sellAmountBeforeFee atoms,
// quoted for the zero address so the read needs no wallet. Orders are
// created from account-scoped drafts under orders/<wallet>/<index>/new,
// which quote for the account's own address.
petal::route_file!(spec: petal::http_read_spec(5_000), read: |ctx: &petal::Ctx| {
    let network = match petal::param(ctx, "network") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let network = match crate::cow::CowNetwork::from_segment(network) {
        Some(network) => network,
        None => return petal::error(-3, "unknown network; expected mainnet, xdai, arbitrum_one or base"),
    };
    let pair = match petal::param(ctx, "pair") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let (sell_token, buy_token) = match crate::cow::params::parse_pair(pair) {
        Ok(pair) => pair,
        Err(e) => return petal::error(-3, format!("pair: {e}")),
    };
    let amount = match petal::param(ctx, "amount") {
        Ok(value) => value,
        Err(resp) => return resp,
    };
    let amount = match crate::cow::params::parse_amount(amount) {
        Ok(amount) => amount,
        Err(e) => return petal::error(-3, format!("amount: {e}")),
    };
    match crate::cow::quote::fetch_quote(&crate::cow::quote::QuoteRequest {
        network,
        sell_token,
        buy_token,
        from: alloy::primitives::Address::ZERO,
        side: crate::cow::quote::QuoteSide::Sell,
        amount,
    }) {
        Ok((_quoted, projected)) => petal::read_json_value(&projected),
        Err(resp) => resp,
    }
});
