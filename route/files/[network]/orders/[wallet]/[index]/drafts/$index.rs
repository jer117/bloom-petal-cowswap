// Draft ids for this account, listed from its private store.
petal::route_file!(spec: petal::store_dir_spec().caps(&["bloom:store"]), ctx_list: |ctx: &petal::Ctx| {
    let network = match petal::param(ctx, "network") {
        Ok(value) => value,
        Err(resp) => return Err(resp),
    };
    let network = match crate::cow::CowNetwork::from_segment(network) {
        Some(network) => network,
        None => {
            return Err(petal::error(
                -3,
                "unknown network; expected mainnet, xdai, arbitrum_one or base",
            ))
        }
    };
    let account = match petal::wallet_account(ctx) {
        Ok(account) => account,
        Err(resp) => return Err(resp),
    };
    let prefix = format!(
        "orders/{}/{}/{}/",
        network.api_env(),
        account.wallet(),
        account.number()
    );
    match petal::sdk::store_list(&prefix, 16 * 1024) {
        Ok(keys) => {
            let mut ids: Vec<String> = keys
                .into_iter()
                .filter_map(|key| {
                    let rest = key.strip_prefix(&prefix)?;
                    let id = rest.strip_suffix("/draft.json")?;
                    crate::cow::params::parse_draft_id(id).ok()?;
                    Some(id.to_string())
                })
                .collect();
            ids.sort();
            ids.dedup();
            Ok(petal::dirs(ids))
        }
        Err(e) => Err(petal::error(-4, format!("list drafts: {}", e.message()))),
    }
});
