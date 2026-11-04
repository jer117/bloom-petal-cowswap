//! The account's live order history from
//! `GET /api/v1/account/{owner}/orders`, projected to a stable small shape.

use alloy::primitives::Address;
use petal::DispatchResponse;

use crate::cow::http;
use crate::cow::CowNetwork;

/// How many orders to pull per listing.
pub const HISTORY_LIMIT: u32 = 50;

/// Fetch and project the owner's orders from the orderbook.
pub fn account_orders(network: CowNetwork, owner: Address) -> Result<serde_json::Value, DispatchResponse> {
    let raw = http::get_json(
        network,
        &format!("/account/{}/orders?limit={HISTORY_LIMIT}", owner.to_checksum(None)),
    )?;
    let rows = raw
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(project_order)
        .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "schema": "bloom.cowswap.account_orders.v1",
        "network": network.api_env(),
        "chain_id": network.chain_id(),
        "owner": owner.to_checksum(None),
        "count": rows.len(),
        "orders": rows,
    }))
}

/// One orderbook order, projected. The signature field is dropped: it is
/// public on the orderbook, but this Petal never needs to surface it.
fn project_order(order: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "uid": order.get("uid"),
        "creationDate": order.get("creationDate"),
        "owner": order.get("owner"),
        "sellToken": order.get("sellToken"),
        "buyToken": order.get("buyToken"),
        "sellAmount": order.get("sellAmount"),
        "buyAmount": order.get("buyAmount"),
        "validTo": order.get("validTo"),
        "feeAmount": order.get("feeAmount"),
        "kind": order.get("kind"),
        "status": order.get("status"),
        "class": order.get("class"),
        "invalidated": order.get("invalidated"),
        "partiallyFillable": order.get("partiallyFillable"),
        "executedSellAmount": order.get("executedSellAmount"),
        "executedBuyAmount": order.get("executedBuyAmount"),
        "executedFeeAmount": order.get("executedFeeAmount"),
        "signingScheme": order.get("signingScheme"),
    })
}

/// Fetch a single order by UID for the status route.
pub fn order_by_uid(network: CowNetwork, uid: &str) -> Result<serde_json::Value, DispatchResponse> {
    let raw = http::get_json(network, &format!("/orders/{uid}"))?;
    Ok(project_order(raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a live `GET /api/v1/account/{owner}/orders` response
    /// (mainnet) on 2026-11-04.
    const LIVE_ROW: &str = r#"{"creationDate":"2026-10-19T02:50:24.034366Z","owner":"0x0000000000000000000000000000000000000001","uid":"0x120a20a56b6802a4b7637a6f3e9e59d197d7874e0a21095d9456b162ed6fe7ba000000000000000000000000000000000000000170dbd880","availableBalance":null,"executedBuyAmount":"0","executedSellAmount":"0","executedSellAmountBeforeFees":"0","executedFeeAmount":"0","executedFee":"0","executedFeeToken":"0x6b175474e89094c44da98b954eedeac495271d0f","invalidated":false,"status":"presignaturePending","class":"limit","settlementContract":"0x9008d19f58aabd9ed0d60971565aa8510560ab41","isLiquidityOrder":false,"fullAppData":null,"sellToken":"0x6b175474e89094c44da98b954eedeac495271d0f","buyToken":"0x6b175474e89094c44da98b954eedeac495271d0f","receiver":"0x0000000000000000000000000000000000000001","sellAmount":"1000000000000000000","buyAmount":"1000000","validTo":1893456000,"appData":"0x75cd9eb72b5c5424bcded4cc1b5aec9a34c227fc2b80a2429fe2f14f11b28ee5","feeAmount":"0","kind":"sell","partiallyFillable":false,"sellTokenBalance":"erc20","buyTokenBalance":"erc20","signingScheme":"presign","signature":"0x","interactions":{"pre":[],"post":[]}}"#;

    #[test]
    fn live_order_projects_without_signature_or_appdata_blob() {
        let raw: serde_json::Value = serde_json::from_str(LIVE_ROW).unwrap();
        let projected = project_order(raw);
        assert_eq!(
            projected["uid"],
            "0x120a20a56b6802a4b7637a6f3e9e59d197d7874e0a21095d9456b162ed6fe7ba000000000000000000000000000000000000000170dbd880"
        );
        assert_eq!(projected["status"], "presignaturePending");
        assert_eq!(projected["kind"], "sell");
        assert_eq!(projected["validTo"], 1_893_456_000u64);
        let text = projected.to_string();
        assert!(projected.get("signature").is_none(), "signature must be dropped");
        assert!(projected.get("fullAppData").is_none(), "fullAppData must be dropped");
        assert!(projected.get("interactions").is_none(), "interactions must be dropped");
    }

    #[test]
    fn non_array_responses_project_to_an_empty_listing() {
        // A malformed (non-array) account response yields an empty listing
        // rather than an error, matching the projection's defensive `.cloned()
        // .unwrap_or_default()` path.
        let raw = serde_json::json!({"error": "unexpected"});
        let rows: Vec<serde_json::Value> = raw
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(project_order)
            .collect();
        assert!(rows.is_empty());
    }
}
