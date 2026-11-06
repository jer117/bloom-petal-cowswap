//! The order draft model: quote-backed, slippage-bounded intents that are
//! stored in the `state` namespace and later signed and posted by the
//! confirm route.
//!
//! A draft pins the quote it was built from and the exact GPv2 order that
//! will be signed, so the reviewable intent and the signed bytes cannot
//! drift apart. No signature material is ever stored.

use alloy::primitives::{Address, U256};
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
use petal::WalletAccount;

use crate::cow::app_data;
use crate::cow::order::Order;
use crate::cow::params;
use crate::cow::quote::{self, QuoteSide, QuotedOrder};
use crate::cow::CowNetwork;

/// Cap on any draft record read back from the store.
const MAX_STORE_BYTES: usize = 64 * 1024;

/// The signing intent under which confirm requests the owner's signature.
pub const SIGNING_INTENT: &str = "cowswap.order.v1";

/// The states a draft moves through.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    /// Quote fetched and order pinned; awaiting a confirm write.
    Quoted,
    /// Bloom reported a pending owner approval for the signing request.
    SigningPending,
    /// Signature obtained and order POST accepted by the orderbook.
    Posted,
    /// POST outcome unknown after signing; next confirm reconciles by UID.
    Ambiguous,
    /// The orderbook refused the order.
    Rejected,
}

/// The reviewable order intent. Immutable once created except for the
/// lifecycle fields (`status`, `approval`, `uid`, `last_error`, `attempts`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OrderDraft {
    pub schema: String,
    pub id: String,
    /// `[network]` segment this draft lives under.
    pub network: String,
    pub chain_id: u64,
    /// Bloom wallet name (trusted, from the resolved `[wallet]` param).
    pub wallet: String,
    /// Bloom account number (trusted, from the resolved `[index]` param).
    pub account: u32,
    /// Checksummed owner address the order will be signed for.
    pub owner: String,
    pub side: QuoteSide,
    pub sell_token: String,
    pub buy_token: String,
    /// The amount the user asked to trade: `sellAmountBeforeFee` for sells,
    /// `buyAmountAfterFee` for buys.
    pub input_amount: String,
    pub slippage_bps: u64,
    pub quote: QuotedOrder,
    /// The order that confirm will sign, already slippage-bounded.
    pub order: SignedOrderView,
    pub status: DraftStatus,
    #[serde(default)]
    pub approval: Option<ApprovalState>,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub created_ms: u64,
}

/// A pending owner approval for this draft's signing request.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalState {
    pub action_id: String,
    pub expires_ms: u64,
}

/// The exact GPv2 order confirm signs, in plain JSON. This is the
/// reviewable "what will be signed" payload; field values are decimal
/// strings or hex exactly as they enter the EIP-712 struct.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SignedOrderView {
    pub sell_token: String,
    pub buy_token: String,
    pub receiver: String,
    pub sell_amount: String,
    pub buy_amount: String,
    pub valid_to: u64,
    pub app_data: String,
    pub app_data_document: String,
    pub fee_amount: String,
    pub kind: String,
    pub partially_fillable: bool,
    pub sell_token_balance: String,
    pub buy_token_balance: String,
}

impl SignedOrderView {
    /// The GPv2 struct with the same fields, for hashing and signing.
    pub fn to_gpv2(&self) -> Result<Order, String> {
        let parse_u256 = |value: &str, name: &str| {
            U256::from_str_radix(value, 10).map_err(|_| format!("order {name} is not decimal"))
        };
        Ok(Order {
            sellToken: self.sell_token.parse().map_err(|_| "sellToken")?,
            buyToken: self.buy_token.parse().map_err(|_| "buyToken")?,
            receiver: self.receiver.parse().map_err(|_| "receiver")?,
            sellAmount: parse_u256(&self.sell_amount, "sellAmount")?,
            buyAmount: parse_u256(&self.buy_amount, "buyAmount")?,
            validTo: u32::try_from(self.valid_to).map_err(|_| "validTo")?,
            appData: self.app_data.parse().map_err(|_| "appData")?,
            feeAmount: parse_u256(&self.fee_amount, "feeAmount")?,
            kind: self.kind.clone(),
            partiallyFillable: self.partially_fillable,
            sellTokenBalance: self.sell_token_balance.clone(),
            buyTokenBalance: self.buy_token_balance.clone(),
        })
    }
}

/// The minimum buy amount a sell order accepts, at `slippage_bps` below the
/// quoted buy amount: `floor(buyAmount * (10000 - bps) / 10000)`.
pub fn min_buy_amount(quoted_buy: U256, slippage_bps: u64) -> U256 {
    let scale = U256::from(10_000u64.saturating_sub(slippage_bps));
    quoted_buy.saturating_mul(scale) / U256::from(10_000u64)
}

/// The maximum sell amount a buy order pays, at `slippage_bps` above the
/// quoted sell amount: `ceil(sellAmount * (10000 + bps) / 10000)`.
pub fn max_sell_amount(quoted_sell: U256, slippage_bps: u64) -> U256 {
    let scale = U256::from(10_000u64.saturating_add(slippage_bps));
    let scaled = quoted_sell.saturating_mul(scale);
    (scaled + U256::from(9_999u64)) / U256::from(10_000u64)
}

/// Build the slippage-bounded order view from a quote.
pub fn order_from_quote(
    quote: &QuotedOrder,
    side: QuoteSide,
    slippage_bps: u64,
) -> Result<SignedOrderView, String> {
    let parse = |value: &str| U256::from_str_radix(value, 10);
    let (sell_amount, buy_amount, kind) = match side {
        QuoteSide::Sell => {
            let quoted_buy = parse(&quote.buy_amount).map_err(|_| "quote buyAmount")?;
            (quote.sell_amount.clone(), min_buy_amount(quoted_buy, slippage_bps).to_string(), "sell")
        }
        QuoteSide::Buy => {
            let quoted_sell = parse(&quote.sell_amount).map_err(|_| "quote sellAmount")?;
            (max_sell_amount(quoted_sell, slippage_bps).to_string(), quote.buy_amount.clone(), "buy")
        }
    };
    if quote.kind != kind {
        return Err(format!(
            "quote kind {} does not match the requested side {kind}",
            quote.kind
        ));
    }
    Ok(SignedOrderView {
        sell_token: quote.sell_token.clone(),
        buy_token: quote.buy_token.clone(),
        receiver: Address::ZERO.to_string(),
        sell_amount,
        buy_amount,
        valid_to: quote.valid_to,
        app_data: format!("{:#x}", app_data::app_data_hash()),
        app_data_document: app_data::app_data_document(),
        fee_amount: quote.fee_amount.clone(),
        kind: kind.to_string(),
        partially_fillable: false,
        sell_token_balance: "erc20".into(),
        buy_token_balance: "erc20".into(),
    })
}

/// The store key for a draft. Account store namespaces are private per
/// Bloom account; the wallet/account/network/id path keeps keys collision
/// free and auditable.
pub fn draft_key(network_segment: &str, wallet: &str, account: u32, id: &str) -> String {
    format!("orders/{network_segment}/{wallet}/{account}/{id}/draft.json")
}

/// Read a draft from the store, mapping a missing key to `not found`.
pub fn load_draft(key: &str) -> Result<OrderDraft, DispatchResponse> {
    let bytes = match petal::sdk::store_get(key, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(SdkError::Host(HostStatus::NotFound)) => {
            return Err(petal::error(-1, "order draft not found"))
        }
        Err(error) => return Err(petal::error(-4, format!("read draft: {}", error.message()))),
    };
    let draft: OrderDraft = serde_json::from_slice(&bytes)
        .map_err(|e| petal::error(-4, format!("draft record is corrupt: {e}")))?;
    if draft.schema != "bloom.cowswap.order_draft.v1" {
        return Err(petal::error(-4, "draft record has an unknown schema"));
    }
    Ok(draft)
}

/// Persist a draft.
pub fn save_draft(key: &str, draft: &OrderDraft) -> Result<(), DispatchResponse> {
    let bytes = serde_json::to_vec(draft)
        .map_err(|e| petal::error(-4, format!("encode draft: {e}")))?;
    petal::sdk::store_put(key, &bytes, false)
        .map_err(|e| petal::error(-4, format!("persist draft: {}", e.message())))
}

/// The parsed side of a draft request body.
#[derive(Debug, serde::Deserialize)]
pub struct NewDraftRequest {
    pub sell_token: String,
    pub buy_token: String,
    pub kind: String,
    /// `sellAmountBeforeFee` for sells, `buyAmountAfterFee` for buys,
    /// decimal atoms.
    pub amount: String,
    #[serde(default)]
    pub slippage_bps: Option<u64>,
}

/// Create a draft: validate the request body, quote it on `network` for the
/// account's address, bound it with slippage, and store it under a fresh id.
pub fn create_draft(
    network: CowNetwork,
    account: &WalletAccount,
    body: &[u8],
) -> DispatchResponse {
    let request: NewDraftRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(e) => return petal::error(-3, format!("new order JSON: {e}")),
    };
    let side = match request.kind.as_str() {
        "sell" => QuoteSide::Sell,
        "buy" => QuoteSide::Buy,
        other => {
            return petal::error(-3, format!("kind must be \"sell\" or \"buy\", got {other:?}"))
        }
    };
    let sell_token = match params::parse_address(&request.sell_token) {
        Ok(address) => address,
        Err(e) => return petal::error(-3, format!("sell_token: {e}")),
    };
    let buy_token = match params::parse_address(&request.buy_token) {
        Ok(address) => address,
        Err(e) => return petal::error(-3, format!("buy_token: {e}")),
    };
    if sell_token == buy_token {
        return petal::error(-3, "sell_token and buy_token must differ");
    }
    let amount = match params::parse_amount(&request.amount) {
        Ok(amount) => amount,
        Err(e) => return petal::error(-3, format!("amount: {e}")),
    };
    let slippage_bps = match request.slippage_bps {
        None => quote::DEFAULT_SLIPPAGE_BPS,
        Some(bps) => match params::parse_slippage_bps(bps) {
            Ok(bps) => bps,
            Err(e) => return petal::error(-3, format!("slippage_bps: {e}")),
        },
    };
    let owner = match account.address_evm() {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    let owner_address = match owner.parse::<Address>() {
        Ok(address) => address,
        Err(_) => return petal::error(-4, "account EVM address is not parseable"),
    };
    let (quoted, _projected) = match quote::fetch_quote(&quote::QuoteRequest {
        network,
        sell_token,
        buy_token,
        from: owner_address,
        side,
        amount,
    }) {
        Ok(result) => result,
        Err(resp) => return resp,
    };
    let order_view = match order_from_quote(&quoted, side, slippage_bps) {
        Ok(view) => view,
        Err(e) => return petal::error(-4, format!("quote did not yield an orderable shape: {e}")),
    };
    let now_ms = petal::sdk::try_now_ms().unwrap_or_default();
    let id = mint_draft_id(network, account, &request, now_ms);
    let draft = OrderDraft {
        schema: "bloom.cowswap.order_draft.v1".into(),
        id: id.clone(),
        network: network.api_env().to_string(),
        chain_id: network.chain_id(),
        wallet: account.wallet().to_string(),
        account: account.number(),
        owner: owner_address.to_checksum(None),
        side,
        sell_token: quoted.sell_token.clone(),
        buy_token: quoted.buy_token.clone(),
        input_amount: amount.to_string(),
        slippage_bps,
        quote: quoted,
        order: order_view,
        status: DraftStatus::Quoted,
        approval: None,
        uid: None,
        last_error: None,
        attempts: 0,
        created_ms: now_ms,
    };
    let key = draft_key(network.api_env(), account.wallet(), account.number(), &id);
    if let Err(resp) = save_draft(&key, &draft) {
        return resp;
    }
    petal::DispatchResponse::Write
}

/// A fresh draft id: `od-` + 16 hex characters derived from the request,
/// the account, and a time+randomness nonce. Hex by construction, so the id
/// is a safe store-key and URL segment without further checks.
fn mint_draft_id(network: CowNetwork, account: &WalletAccount, request: &NewDraftRequest, now_ms: u64) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    network.api_env().hash(&mut hasher);
    account.wallet().hash(&mut hasher);
    account.number().hash(&mut hasher);
    request.sell_token.hash(&mut hasher);
    request.buy_token.hash(&mut hasher);
    request.kind.hash(&mut hasher);
    request.amount.hash(&mut hasher);
    now_ms.hash(&mut hasher);
    let entropy = petal::sdk::random_bytes(8).unwrap_or_default();
    hasher.write(&entropy);
    format!("od-{:016x}", hasher.finish())
}

/// The public, reviewable projection of a draft: everything the owner or an
/// agent needs to decide, and nothing signature-like.
pub fn project_draft(draft: &OrderDraft) -> serde_json::Value {
    serde_json::json!({
        "schema": draft.schema,
        "id": draft.id,
        "network": draft.network,
        "chain_id": draft.chain_id,
        "wallet": draft.wallet,
        "account": draft.account,
        "owner": draft.owner,
        "side": draft.side.as_str(),
        "input_amount": draft.input_amount,
        "slippage_bps": draft.slippage_bps,
        "status": draft.status,
        "quote": {
            "sell_amount": draft.quote.sell_amount,
            "buy_amount": draft.quote.buy_amount,
            "fee_amount": draft.quote.fee_amount,
            "valid_to": draft.quote.valid_to,
            "kind": draft.quote.kind,
        },
        "order_to_sign": draft.order,
        "what_will_be_signed": {
            "summary": format!(
                "EIP-712 GPv2 {} order: sell {} ({}), buy {} ({}), fee {}, validTo {}",
                draft.order.kind,
                draft.order.sell_amount,
                draft.order.sell_token,
                draft.order.buy_amount,
                draft.order.buy_token,
                draft.order.fee_amount,
                draft.order.valid_to,
            ),
            "domain": {
                "name": "Gnosis Protocol",
                "version": "v2",
                "chainId": draft.chain_id,
                "verifyingContract": crate::cow::network::SETTLEMENT_CONTRACT,
            },
        },
        "approval_requirement": {
            "spender": crate::cow::network::VAULT_RELAYER,
            "token": draft.order.sell_token,
            "note": "ERC20 allowance to the GPv2 Vault Relayer must cover sellAmount + feeAmount; see the allowance route",
        },
        "approval": draft.approval,
        "uid": draft.uid,
        "last_error": draft.last_error,
        "attempts": draft.attempts,
        "created_ms": draft.created_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted(kind: &str) -> QuotedOrder {
        QuotedOrder {
            sell_token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            buy_token: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".into(),
            sell_amount: "9_999_312_248".replace('_', ""),
            buy_amount: "4_308_666_162_407_131_131".replace('_', ""),
            valid_to: 1_793_804_915,
            fee_amount: "687_752".replace('_', ""),
            kind: kind.into(),
        }
    }

    #[test]
    fn sell_orders_floor_the_buy_amount() {
        // 4308666162407131131 * (1 - 0.005) = 4308666162407131131 * 9950/10000
        let quoted_buy = U256::from(4_308_666_162_407_131_131u128);
        let floor = min_buy_amount(quoted_buy, 50);
        assert_eq!(floor, quoted_buy * U256::from(9950u64) / U256::from(10_000u64));
        // Exactness of the floor: multiply then divide, no premature rounding.
        assert_eq!(min_buy_amount(U256::from(10_000u64), 5_000), U256::from(5_000u64));
        assert_eq!(min_buy_amount(U256::from(10_001u64), 5_000), U256::from(5_000u64));
        assert_eq!(min_buy_amount(U256::from(10_000u64), 1), U256::from(9_999u64));
        // Full-range slippage clamps to zero without underflow.
        assert_eq!(min_buy_amount(U256::from(10_000u64), 10_000), U256::ZERO);
    }

    #[test]
    fn buy_orders_ceil_the_sell_amount() {
        let quoted_sell = U256::from(10_000u64);
        // ceil(10000 * 10050/10000) = 10050 exactly.
        assert_eq!(max_sell_amount(quoted_sell, 50), U256::from(10_050u64));
        // ceil(9999 * 10050/10000) = ceil(10048.995) = 10049.
        assert_eq!(max_sell_amount(U256::from(9_999u64), 50), U256::from(10_049u64));
        // Worst case stays bounded.
        assert_eq!(max_sell_amount(U256::from(1u64), 5_000), U256::from(2u64));
    }

    #[test]
    fn sell_order_view_carries_the_quote_and_bounds() {
        let view = order_from_quote(&quoted("sell"), QuoteSide::Sell, 50).unwrap();
        assert_eq!(view.kind, "sell");
        assert_eq!(view.sell_amount, "9999312248");
        assert_eq!(
            view.buy_amount,
            (U256::from(4_308_666_162_407_131_131u128) * U256::from(9_950u64)
                / U256::from(10_000u64))
            .to_string()
        );
        assert_eq!(view.fee_amount, "687752");
        assert_eq!(view.valid_to, 1_793_804_915);
        assert!(!view.partially_fillable);
        assert_eq!(view.sell_token_balance, "erc20");
        assert_eq!(view.receiver, "0x0000000000000000000000000000000000000000");
        assert_eq!(view.app_data_document, app_data::app_data_document());
        // appData hash is keccak256 of the document bytes.
        assert_eq!(
            view.app_data,
            format!("{:#x}", app_data::app_data_hash())
        );
        // The view round-trips into the GPv2 struct the confirm route signs.
        let gpv2 = view.to_gpv2().unwrap();
        assert_eq!(gpv2.kind, "sell");
    }

    #[test]
    fn buy_order_view_swaps_the_bound() {
        let view = order_from_quote(&quoted("buy"), QuoteSide::Buy, 100).unwrap();
        assert_eq!(view.kind, "buy");
        assert_eq!(view.buy_amount, "4308666162407131131");
        // ceil(9999312248 * 10100 / 10000)
        let expected = (U256::from(9_999_312_248u64) * U256::from(10_100u64)
            + U256::from(9_999u64))
            / U256::from(10_000u64);
        assert_eq!(view.sell_amount, expected.to_string());
    }

    #[test]
    fn mismatched_quote_kind_is_refused() {
        assert!(order_from_quote(&quoted("buy"), QuoteSide::Sell, 50).is_err());
        assert!(order_from_quote(&quoted("sell"), QuoteSide::Buy, 50).is_err());
    }

    #[test]
    fn draft_keys_are_structured_and_bounded() {
        assert_eq!(
            draft_key("mainnet", "kaze", 0, "od-0123456789abcdef"),
            "orders/mainnet/kaze/0/od-0123456789abcdef/draft.json"
        );
    }

    #[test]
    fn draft_projection_never_leaks_signature_material() {
        let mut draft = OrderDraft {
            schema: "bloom.cowswap.order_draft.v1".into(),
            id: "od-0123456789abcdef".into(),
            network: "mainnet".into(),
            chain_id: 1,
            wallet: "kaze".into(),
            account: 0,
            owner: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            side: QuoteSide::Sell,
            sell_token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            buy_token: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".into(),
            input_amount: "10000000000".into(),
            slippage_bps: 50,
            quote: quoted("sell"),
            order: order_from_quote(&quoted("sell"), QuoteSide::Sell, 50).unwrap(),
            status: DraftStatus::Quoted,
            approval: None,
            uid: None,
            last_error: None,
            attempts: 0,
            created_ms: 1,
        };
        let projected = project_draft(&draft).to_string();
        assert!(!projected.contains("signature"), "no signature fields");
        assert!(projected.contains("order_to_sign"));
        assert!(projected.contains("approval_requirement"));
        assert!(projected.contains(SETTLEMENT_LOOK), "domain is reviewable");
        draft.status = DraftStatus::SigningPending;
        draft.approval = Some(ApprovalState { action_id: "act-1".into(), expires_ms: 9 });
        let projected = project_draft(&draft);
        assert_eq!(projected["approval"]["action_id"], "act-1");
        assert_eq!(projected["status"], "signing_pending");
    }

    const SETTLEMENT_LOOK: &str = "0x9008D19f58AAbD9eD0D60971565AA8510560ab41";

    #[test]
    fn draft_round_trips_through_json() {
        let draft = OrderDraft {
            schema: "bloom.cowswap.order_draft.v1".into(),
            id: "od-0123456789abcdef".into(),
            network: "xdai".into(),
            chain_id: 100,
            wallet: "kaze".into(),
            account: 2,
            owner: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            side: QuoteSide::Buy,
            sell_token: "0xe91d153e0b41518a2ce8dd3d7944fa863463a97d".into(),
            buy_token: "0x177127622c4a00f06dd6e2a75413a67a3d0db41b".into(),
            input_amount: "1000000000000000000".into(),
            slippage_bps: 100,
            quote: quoted("buy"),
            order: order_from_quote(&quoted("buy"), QuoteSide::Buy, 100).unwrap(),
            status: DraftStatus::Posted,
            approval: None,
            uid: Some(format!("0x{}", "ab".repeat(56))),
            last_error: None,
            attempts: 1,
            created_ms: 42,
        };
        let bytes = serde_json::to_vec(&draft).unwrap();
        let back: OrderDraft = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back, draft);
        assert_eq!(back.status, DraftStatus::Posted);
    }
}
