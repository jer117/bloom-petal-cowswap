// drafts/<id>/status.json — local draft state merged with the live
// orderbook view of the posted order (GET /api/v1/orders/{uid}).
//
// The order's UID is fixed by the order, so once a confirm has been
// attempted this looks the order up even if the draft never recorded a UID
// (a lost response or a failed store write). Not cached (TTL 0), so the
// agent sees a confirm's outcome at once.
petal::route_file!(spec: petal::http_read_spec(0).caps(&["bloom:http", "bloom:store"]), read: |ctx: &petal::Ctx| {
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
    let draft = match crate::cow::draft::load_draft(&key) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    let recorded = draft
        .uid
        .clone()
        .filter(|uid| crate::cow::params::parse_order_uid(uid).is_ok());
    let local = |posted: bool, uid: Option<&str>, order: serde_json::Value, note: Option<String>| {
        petal::read_json_value(&serde_json::json!({
            "schema": "bloom.cowswap.order_status.v1",
            "id": draft.id,
            "network": draft.network,
            "status": draft.status,
            "posted": posted,
            "uid": uid,
            "order": order,
            "upstream_note": note,
            "approval": draft.approval,
            "last_error": draft.last_error,
            "attempts": draft.attempts,
        }))
    };
    let uid = match &recorded {
        Some(uid) => uid.clone(),
        // Never confirmed: the local state is the whole story.
        None if draft.status == crate::cow::draft::DraftStatus::Quoted => {
            return local(false, None, serde_json::Value::Null, None);
        }
        None => match draft.expected_uid(network) {
            Ok(uid) => uid,
            Err(_) => return local(false, None, serde_json::Value::Null, None),
        },
    };
    match crate::cow::account_orders::find_order(network, &uid) {
        Ok(Some(order)) => local(true, Some(&uid), order, None),
        // The orderbook may briefly not show a just-accepted order. Without a
        // recorded UID, an order it does not show was not posted.
        Ok(None) if recorded.is_some() => local(
            true,
            Some(&uid),
            serde_json::Value::Null,
            Some("the orderbook does not show this order yet".into()),
        ),
        Ok(None) => local(false, None, serde_json::Value::Null, None),
        Err(petal::DispatchResponse::Error { code: -4, message }) => local(
            recorded.is_some(),
            recorded.as_deref(),
            serde_json::Value::Null,
            Some(message),
        ),
        Err(resp) => resp,
    }
});
