//! The appData document this Petal attaches to every order, and its
//! keccak256 hash as it goes into the signed GPv2 order struct.
//!
//! The full appData JSON string is sent with the order creation request; the
//! backend derives the same hash from those exact bytes. The document follows
//! the CoW app-data schema (only `version` and `metadata` are required;
//! `appCode` names the integrator honestly).

use alloy::primitives::{B256, keccak256};

/// The appData schema version the document targets.
pub const APP_DATA_VERSION: &str = "1.16.0";

/// Honest integrator identification for orders placed through this Petal.
pub const APP_CODE: &str = "bloom-cowswap-petal";

/// Serialize the full appData document deterministically. `serde_json`'s
/// object map keeps keys sorted, so the bytes — and therefore the hash — are
/// stable across builds.
pub fn app_data_document() -> String {
    serde_json::json!({
        "appCode": APP_CODE,
        "environment": "production",
        "metadata": {},
        "version": APP_DATA_VERSION,
    })
    .to_string()
}

/// The `appData` field of the signed GPv2 order: keccak256 over the UTF-8
/// bytes of the full document string.
pub fn app_data_hash() -> B256 {
    keccak256(app_data_document().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// keccak256("cow"), as published in the CoW docs' signing examples.
    #[test]
    fn keccak_matches_the_documented_cow_vector() {
        assert_eq!(
            keccak256(b"cow").to_string(),
            "0xc85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4"
        );
    }

    /// The documented hash of the empty appData object "{}", which the
    /// orderbook API itself cites as the "do not care" appData.
    #[test]
    fn keccak_matches_the_documented_empty_appdata_vector() {
        assert_eq!(
            keccak256(b"{}").to_string(),
            "0xb48d38f93eaa084033fc5970bf96e559c33c4cdc07d889ab00b4d63f9590739d"
        );
    }

    /// The document is a valid JSON object with the schema-required keys, and
    /// its exact bytes pin the signed appData hash.
    #[test]
    fn document_is_deterministic_and_schema_shaped() {
        let document = app_data_document();
        let value: serde_json::Value =
            serde_json::from_str(&document).expect("document must be JSON");
        let object = value.as_object().expect("document must be an object");
        assert_eq!(object["appCode"], APP_CODE);
        assert_eq!(object["version"], APP_DATA_VERSION);
        assert_eq!(object["metadata"], serde_json::json!({}));
        // Sorted-key serialization keeps the bytes stable.
        assert_eq!(app_data_document(), document);
        // The pinned hash was computed independently (Python keccak via a
        // JSON-RPC node) over the exact document bytes; update this constant
        // deliberately when the document changes.
        assert_eq!(
            app_data_hash().to_string(),
            "0xb59ec5fad15447b301bafeef096c7874d24b4a80611f9ecefd532b0ecccf4326"
        );
    }
}
