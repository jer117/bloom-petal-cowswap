// allowance/<wallet>/<index>/<0xTOKEN>.json
// The account's ERC20 allowance to the GPv2 Vault Relayer for TOKEN, plus
// its token balance, read from chain. CoW orders pull the sell token
// through the Vault Relayer, so an order needs both the allowance and the
// balance to be at least its sell_amount.
//
// Not cached (TTL 0): an agent reads this again right after an approve
// transaction, and a stale answer invites a second one.
petal::route_file!(
    spec: petal::http_read_spec(0).caps(&["bloom:chain", "bloom:vfs.read"]),
    read: |ctx: &petal::Ctx| {
        let network = match petal::param(ctx, "network") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let network = match crate::cow::CowNetwork::from_segment(network) {
            Some(network) => network,
            None => return petal::error(-3, "unknown network; expected mainnet, xdai, arbitrum_one, base or sepolia"),
        };
        let token = match petal::param(ctx, "token") {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let token = match crate::cow::params::parse_address(token) {
            Ok(token) => token,
            Err(e) => return petal::error(-3, format!("token: {e}")),
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
        if let Err(resp) = crate::cow::chain::verify_chain_route(network) {
            return resp;
        }
        let spender = match crate::cow::network::VAULT_RELAYER.parse::<alloy::primitives::Address>() {
            Ok(address) => address,
            Err(_) => return petal::error(-4, "vault relayer constant is invalid"),
        };
        let allowance = match crate::cow::chain::read_allowance(network, token, owner, spender) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        let balance = match crate::cow::chain::read_balance(network, token, owner) {
            Ok(value) => value,
            Err(resp) => return resp,
        };
        petal::read_json_value(&serde_json::json!({
            "schema": "bloom.cowswap.allowance.v1",
            "network": network.api_env(),
            "chain_id": network.chain_id(),
            "owner": owner.to_checksum(None),
            "wallet": account.wallet(),
            "account": account.number(),
            "token": format!("{token:#x}"),
            "spender": crate::cow::network::VAULT_RELAYER,
            "spender_kind": "GPv2VaultRelayer",
            "allowance": allowance.to_string(),
            "balance": balance.to_string(),
            "requirement": "an order needs allowance and balance of at least its order_to_sign.sell_amount",
        }))
    }
);
