//! Bounded HTTP adapter for the CoW Protocol orderbook API.
//!
//! Targets are built only from a [`CowNetwork`] plus a literal path suffix,
//! so route code can never direct a request at an arbitrary host. Upstream
//! errors are truncated and reported as backend failures without echoing
//! arbitrary response bodies.

use petal::sdk::{DispatchResponse, HttpRequest, SdkError};

use crate::cow::CowNetwork;

/// Cap on any response body accepted from the API.
pub const MAX_HTTP_BYTES: usize = 256 * 1024;

/// `https://api.cow.fi/<env>/api/v1` for `network`.
pub fn api_root(network: CowNetwork) -> String {
    format!("{}/{}/api/v1", crate::cow::network::API_BASE, network.api_env())
}

fn request(method: &str, url: &str, body: Vec<u8>) -> Result<petal::sdk::HttpResponse, DispatchResponse> {
    petal::sdk::http_fetch(
        &HttpRequest {
            method: method.into(),
            url: url.into(),
            headers: vec![("content-type".into(), "application/json".into())],
            body,
        },
        MAX_HTTP_BYTES,
    )
    .map_err(sdk_error)
}

/// Map SDK/host failures to route errors, with Bloom's reason when it gave one.
pub fn sdk_error(error: SdkError) -> DispatchResponse {
    petal::error(
        -4,
        format!("orderbook request failed: {}", petal::sdk::describe_error(&error)),
    )
}

/// Issue a GET and parse the JSON body. Non-2xx becomes a sanitized `-4`
/// error carrying only the status and a short excerpt.
pub fn get_json(network: CowNetwork, path: &str) -> Result<serde_json::Value, DispatchResponse> {
    let url = format!("{}{}", api_root(network), path);
    let response = request("GET", &url, Vec::new())?;
    if !(200..300).contains(&response.status) {
        return Err(petal::error(
            -4,
            format!(
                "cow api error (status {}) on GET {}",
                response.status,
                excerpt(path, &response.body)
            ),
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|e| petal::error(-4, format!("cow api response JSON: {e}")))
}

/// Issue a POST with a JSON body and parse the JSON response.
pub fn post_json(
    network: CowNetwork,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, DispatchResponse> {
    let url = format!("{}{}", api_root(network), path);
    let encoded = serde_json::to_vec(body)
        .map_err(|e| petal::error(-4, format!("encode request JSON: {e}")))?;
    let response = request("POST", &url, encoded)?;
    if !(200..300).contains(&response.status) {
        return Err(petal::error(
            -4,
            format!(
                "cow api error (status {}) on POST {}",
                response.status,
                excerpt(path, &response.body)
            ),
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|e| petal::error(-4, format!("cow api response JSON: {e}")))
}

/// The orderbook's answer to an order POST.
#[derive(Debug)]
pub enum OrderPost {
    /// A 2xx: the orderbook created the order. The body is its UID.
    Created(serde_json::Value),
    /// A 4xx: the orderbook did not create the order. `error_type` is the
    /// API's `errorType`, such as `NonZeroFee` or `DuplicatedOrder`.
    Refused {
        status: u16,
        error_type: Option<String>,
        message: String,
    },
}

/// POST an order to `/orders`. A transport failure, a 5xx or an unreadable
/// 2xx body is an error: the order may or may not exist.
pub fn post_order(network: CowNetwork, body: &serde_json::Value) -> Result<OrderPost, DispatchResponse> {
    let url = format!("{}/orders", api_root(network));
    let encoded = serde_json::to_vec(body)
        .map_err(|e| petal::error(-4, format!("encode request JSON: {e}")))?;
    let response = request("POST", &url, encoded)?;
    let message = || {
        format!(
            "cow api error (status {}) on POST {}",
            response.status,
            excerpt("/orders", &response.body)
        )
    };
    match response.status {
        200..=299 => serde_json::from_slice(&response.body)
            .map(OrderPost::Created)
            .map_err(|e| petal::error(-4, format!("cow api response JSON: {e}"))),
        400..=499 => Ok(OrderPost::Refused {
            status: response.status,
            error_type: error_type(&response.body),
            message: message(),
        }),
        _ => Err(petal::error(-4, message())),
    }
}

/// GET `/orders/<uid>`. `None` when the orderbook answers 404: it has no
/// order with that UID.
pub fn get_order(network: CowNetwork, uid: &str) -> Result<Option<serde_json::Value>, DispatchResponse> {
    let path = format!("/orders/{uid}");
    let response = request("GET", &format!("{}{path}", api_root(network)), Vec::new())?;
    match response.status {
        200..=299 => serde_json::from_slice(&response.body)
            .map(Some)
            .map_err(|e| petal::error(-4, format!("cow api response JSON: {e}"))),
        404 => Ok(None),
        status => Err(petal::error(
            -4,
            format!("cow api error (status {status}) on GET {}", excerpt(&path, &response.body)),
        )),
    }
}

/// The `errorType` of an orderbook error body, when it is a plain
/// identifier.
fn error_type(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let kind = value.get("errorType")?.as_str()?;
    (!kind.is_empty() && kind.len() <= 64 && kind.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then(|| kind.to_string())
}

/// A short, sanitized excerpt of an upstream error body: no control
/// characters, bounded length, prefixed by the request path for context.
fn excerpt(path: &str, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    let mut excerpt: String = cleaned.chars().take(160).collect();
    if cleaned.chars().count() > 160 {
        excerpt.push('…');
    }
    if excerpt.is_empty() {
        excerpt.push_str("(no body)");
    }
    format!("{path}: {excerpt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_roots_pin_the_documented_environments() {
        assert_eq!(
            api_root(CowNetwork::Mainnet),
            "https://api.cow.fi/mainnet/api/v1"
        );
        assert_eq!(api_root(CowNetwork::Gnosis), "https://api.cow.fi/xdai/api/v1");
        assert_eq!(
            api_root(CowNetwork::Arbitrum),
            "https://api.cow.fi/arbitrum_one/api/v1"
        );
        assert_eq!(api_root(CowNetwork::Base), "https://api.cow.fi/base/api/v1");
        assert_eq!(
            api_root(CowNetwork::Sepolia),
            "https://api.cow.fi/sepolia/api/v1"
        );
    }

    #[test]
    fn excerpts_are_bounded_and_sanitized() {
        let long = vec![b'a'; 500];
        let shown = excerpt("/api/v1/x", &long);
        assert!(shown.starts_with("/api/v1/x: "));
        assert!(shown.chars().count() <= 160 + 16);
        assert!(shown.ends_with('…'));
        assert_eq!(excerpt("/api/v1/x", b""), "/api/v1/x: (no body)");
        assert_eq!(excerpt("/p", b"a\nb\x00c"), "/p: a b c");
    }

    #[test]
    fn error_types_are_plain_identifiers() {
        assert_eq!(
            error_type(br#"{"errorType":"NonZeroFee","description":"Fee must be zero"}"#),
            Some("NonZeroFee".into())
        );
        assert_eq!(error_type(br#"{"errorType":"Bad Type"}"#), None);
        assert_eq!(error_type(br#"{"errorType":""}"#), None);
        assert_eq!(error_type(br#"{"description":"no type"}"#), None);
        assert_eq!(error_type(b"not json"), None);
    }
}
