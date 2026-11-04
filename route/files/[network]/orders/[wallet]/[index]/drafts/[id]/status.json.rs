// drafts/<id>/status.json — local draft state merged with the live
// orderbook view of the posted order (GET /api/v1/orders/{uid}).
petal::route_file!(spec: petal::account_read_spec().caps(&["bloom:http", "bloom:store"]), read: |ctx: &petal::Ctx| {
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
    let draft = match crate::cow::draft::load_draft(&key) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    let uid = match draft.uid.as_deref() {
        Some(uid) if crate::cow::params::parse_order_uid(uid).is_ok() => uid.to_string(),
        _ => {
            // Not posted yet: the local state is the whole story.
            return petal::read_json_value(&serde_json::json!({
                "schema": "bloom.cowswap.order_status.v1",
                "id": draft.id,
                "network": draft.network,
                "status": draft.status,
                "posted": false,
                "uid": serde_json::Value::Null,
                "last_error": draft.last_error,
                "attempts": draft.attempts,
            }));
        }
    };
    match crate::cow::account_orders::order_by_uid(network, &uid) {
        Ok(order) => petal::read_json_value(&serde_json::json!({
            "schema": "bloom.cowswap.order_status.v1",
            "id": draft.id,
            "network": draft.network,
            "status": draft.status,
            "posted": true,
            "uid": uid,
            "order": order,
            "last_error": draft.last_error,
            "attempts": draft.attempts,
        })),
        Err(petal::DispatchResponse::Error { code: -4, message }) => {
            // The orderbook may briefly 404 a just-accepted order; report the
            // local state instead of failing the read.
            petal::read_json_value(&serde_json::json!({
                "schema": "bloom.cowswap.order_status.v1",
                "id": draft.id,
                "network": draft.network,
                "status": draft.status,
                "posted": true,
                "uid": uid,
                "order": serde_json::Value::Null,
                "upstream_note": message,
                "last_error": draft.last_error,
                "attempts": draft.attempts,
            }))
        }
        Err(resp) => resp,
    }
});
