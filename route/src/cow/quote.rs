//! Quote request construction and response projection for
//! `POST /api/v1/quote`.
//!
//! Shapes follow the orderbook OpenAPI (`OrderQuoteRequest`,
//! `OrderQuoteResponse`, `OrderParameters`); live responses were captured
//! from `api.cow.fi` while writing this Petal (see DESIGN.md).

use alloy::primitives::{Address, U256};
use petal::DispatchResponse;

use crate::cow::app_data;
use crate::cow::http;
use crate::cow::CowNetwork;

/// How long quoted orders stay valid. Relative validity (`validFor`) is
/// requested; the API returns the absolute `validTo`, which the draft pins
/// and the signed order reuses.
pub const QUOTE_VALID_FOR_SECONDS: u64 = 1800;

/// Default slippage bound applied when a draft request does not set one:
/// 50 bps (0.5%).
pub const DEFAULT_SLIPPAGE_BPS: u64 = 50;

/// The quoted order parameters this Petal carries into a draft. Amounts are
/// kept as strings exactly as the API returns them (decimal u256 atoms).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QuotedOrder {
    pub sell_token: String,
    pub buy_token: String,
    pub sell_amount: String,
    pub buy_amount: String,
    pub valid_to: u64,
    pub fee_amount: String,
    pub kind: String,
}

/// Which side a quote is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum QuoteSide {
    Sell,
    Buy,
}

impl QuoteSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sell => "sell",
            Self::Buy => "buy",
        }
    }
}

/// A quote request: everything needed to build the POST body.
pub struct QuoteRequest {
    pub network: CowNetwork,
    pub sell_token: Address,
    pub buy_token: Address,
    pub from: Address,
    pub side: QuoteSide,
    /// `sellAmountBeforeFee` for sell quotes, `buyAmountAfterFee` for buy
    /// quotes.
    pub amount: U256,
}

fn quote_body(request: &QuoteRequest) -> serde_json::Value {
    let amount = request.amount.to_string();
    let side_field = match request.side {
        QuoteSide::Sell => ("sellAmountBeforeFee", amount),
        QuoteSide::Buy => ("buyAmountAfterFee", amount),
    };
    serde_json::json!({
        "sellToken": format!("{:#x}", request.sell_token),
        "buyToken": format!("{:#x}", request.buy_token),
        "from": format!("{:#x}", request.from),
        "kind": request.side.as_str(),
        side_field.0: side_field.1,
        "validFor": QUOTE_VALID_FOR_SECONDS,
        "appData": app_data::app_data_document(),
        "priceQuality": "verified",
    })
}

/// Request a quote and keep the fields the draft flow needs, together with
/// the projected public response.
pub fn fetch_quote(
    request: &QuoteRequest,
) -> Result<(QuotedOrder, serde_json::Value), DispatchResponse> {
    let response = http::post_json(request.network, "/quote", &quote_body(request))?;
    let quote = response
        .get("quote")
        .ok_or_else(|| petal::error(-4, "quote response is missing its quote object"))?;
    let quoted = QuotedOrder {
        sell_token: string_field(quote, "sellToken")?,
        buy_token: string_field(quote, "buyToken")?,
        sell_amount: string_field(quote, "sellAmount")?,
        buy_amount: string_field(quote, "buyAmount")?,
        valid_to: quote
            .get("validTo")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| petal::error(-4, "quote response is missing validTo"))?,
        fee_amount: string_field(quote, "feeAmount")?,
        kind: string_field(quote, "kind")?,
    };
    Ok((quoted, project_quote_response(&response)))
}

fn string_field(value: &serde_json::Value, name: &str) -> Result<String, DispatchResponse> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| petal::error(-4, format!("quote response is missing {name}")))
}

/// Project a raw `/quote` response into a stable, smaller view: what was
/// quoted, the fee, the pre-fee amount, and quote metadata.
pub fn project_quote_response(response: &serde_json::Value) -> serde_json::Value {
    let quote = response.get("quote").cloned().unwrap_or(serde_json::json!({}));
    let parse = |name: &str| {
        U256::from_str_radix(
            quote.get(name).and_then(serde_json::Value::as_str).unwrap_or("0"),
            10,
        )
        .unwrap_or_default()
    };
    let before_fee = parse("sellAmount") + parse("feeAmount");
    serde_json::json!({
        "schema": "bloom.cowswap.quote.v1",
        "quote": {
            "sellToken": quote.get("sellToken"),
            "buyToken": quote.get("buyToken"),
            "receiver": quote.get("receiver"),
            "sellAmount": quote.get("sellAmount"),
            "buyAmount": quote.get("buyAmount"),
            "validTo": quote.get("validTo"),
            "appData": quote.get("appData"),
            "feeAmount": quote.get("feeAmount"),
            "kind": quote.get("kind"),
            "partiallyFillable": quote.get("partiallyFillable"),
            "sellTokenBalance": quote.get("sellTokenBalance"),
            "buyTokenBalance": quote.get("buyTokenBalance"),
        },
        "sellAmountBeforeFee": before_fee.to_string(),
        "from": response.get("from"),
        "id": response.get("id"),
        "expiration": response.get("expiration"),
        "verified": response.get("verified"),
        "protocolFeeBps": response.get("protocolFeeBps"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captured from a live `POST /api/v1/quote` response (mainnet,
    /// USDC→WETH sell, sellAmountBeforeFee 1e10) on 2026-11-04.
    const LIVE_RESPONSE: &str = r#"{"quote":{"sellToken":"0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48","buyToken":"0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2","receiver":"0x0000000000000000000000000000000000000000","sellAmount":"9999312248","buyAmount":"4308666162407131131","validTo":1793804915,"appData":"0x0000000000000000000000000000000000000000000000000000000000000000","feeAmount":"687752","gasAmount":"500000","gasPrice":"592976521","sellTokenPrice":"431098168.222066700458526611328125","kind":"sell","partiallyFillable":false,"sellTokenBalance":"erc20","buyTokenBalance":"erc20","signingScheme":"eip712"},"from":"0x0000000000000000000000000000000000000001","expiration":"1970-01-01T00:00:00Z","id":null,"verified":false,"protocolFeeBps":"2"}"#;

    fn request() -> QuoteRequest {
        QuoteRequest {
            network: CowNetwork::Mainnet,
            sell_token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".parse().unwrap(),
            buy_token: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".parse().unwrap(),
            from: "0x0000000000000000000000000000000000000001".parse().unwrap(),
            side: QuoteSide::Sell,
            amount: U256::from(10_000_000_000u64),
        }
    }

    #[test]
    fn body_carries_the_documented_request_fields() {
        let body = quote_body(&request());
        assert_eq!(body["sellToken"], "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        assert_eq!(body["buyToken"], "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2");
        assert_eq!(body["kind"], "sell");
        assert_eq!(body["sellAmountBeforeFee"], "10000000000");
        assert_eq!(body["validFor"], QUOTE_VALID_FOR_SECONDS);
        assert_eq!(body["priceQuality"], "verified");
        assert_eq!(body["appData"], app_data::app_data_document());
        let buy = quote_body(&QuoteRequest { side: QuoteSide::Buy, ..request() });
        assert!(buy.get("sellAmountBeforeFee").is_none());
        assert_eq!(buy["buyAmountAfterFee"], "10000000000");
        assert_eq!(buy["kind"], "buy");
    }

    #[test]
    fn live_response_projects_to_the_public_shape() {
        let raw: serde_json::Value = serde_json::from_str(LIVE_RESPONSE).unwrap();
        let view = project_quote_response(&raw);
        assert_eq!(view["schema"], "bloom.cowswap.quote.v1");
        assert_eq!(view["quote"]["sellAmount"], "9999312248");
        assert_eq!(view["quote"]["buyAmount"], "4308666162407131131");
        assert_eq!(view["quote"]["feeAmount"], "687752");
        assert_eq!(view["quote"]["validTo"], 1_793_804_915u64);
        assert_eq!(view["quote"]["kind"], "sell");
        // sellAmount(9999312248) + feeAmount(687752) = the pre-fee input.
        assert_eq!(view["sellAmountBeforeFee"], "10000000000");
        assert_eq!(view["verified"], false);
    }

    #[test]
    fn missing_fields_are_backend_errors() {
        assert!(string_field(&serde_json::json!({}), "sellAmount").is_err());
        let bad = serde_json::json!({"quote": {"sellToken": "0x0", "buyToken": "0x0", "sellAmount": "1", "buyAmount": "1", "feeAmount": "0", "kind": "sell"}});
        assert!(string_field(bad.get("quote").unwrap(), "validTo_u64_missing").is_err());
    }
}
