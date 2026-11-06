petal::route_file!(spec: petal::static_read_spec(), read: |_ctx: &petal::Ctx| {
    petal::read_json_value(&serde_json::json!({
        "schema": "bloom.cowswap.status.v1",
        "petal": "cowswap",
        "status": "ok",
        "kind": "cow-protocol-orderbook",
        "networks": [
            {
                "segment": "mainnet",
                "chain_id": 1,
                "bloom_chain": "ethereum",
                "api": "https://api.cow.fi/mainnet/api/v1/",
            },
            {
                "segment": "xdai",
                "chain_id": 100,
                "bloom_chain": "gnosis",
                "api": "https://api.cow.fi/xdai/api/v1/",
            },
            {
                "segment": "arbitrum_one",
                "chain_id": 42161,
                "bloom_chain": "arbitrum",
                "api": "https://api.cow.fi/arbitrum_one/api/v1/",
            },
            {
                "segment": "base",
                "chain_id": 8453,
                "bloom_chain": "base",
                "api": "https://api.cow.fi/base/api/v1/",
            },
            {
                "segment": "sepolia",
                "chain_id": 11155111,
                "bloom_chain": "sepolia",
                "api": "https://api.cow.fi/sepolia/api/v1/",
            }
        ],
        "contracts": {
            // Same address on every supported chain (CoW deployment table).
            "GPv2Settlement": crate::cow::network::SETTLEMENT_CONTRACT,
            "GPv2VaultRelayer": crate::cow::network::VAULT_RELAYER,
        },
        "signing": {
            "intent": crate::cow::draft::SIGNING_INTENT,
            "domain": {
                "name": "Gnosis Protocol",
                "version": "v2",
                "verifyingContract": crate::cow::network::SETTLEMENT_CONTRACT,
            },
        },
        "app_data": {
            "app_code": crate::cow::app_data::APP_CODE,
            "version": crate::cow::app_data::APP_DATA_VERSION,
            "hash": format!("{:#x}", crate::cow::app_data::app_data_hash()),
        },
        "notes": [
            "Reads are unauthenticated orderbook queries; writes create reviewable order drafts, then sign and post them after owner approval.",
            "No API token metadata endpoint exists in the current orderbook API (GET /api/v1/token/{token} returns 404); token decimals are not served by this Petal.",
            "No REST allowance endpoint exists in the current orderbook API; allowance/{wallet}/{index}/{token}.json reads ERC20 allowances from chain.",
        ],
    }))
});
