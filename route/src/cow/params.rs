//! Strict parameter grammars for values that reach URLs, store keys, or
//! chain calls.
//!
//! `petal::is_safe_segment` is a path-safety check, not URL or domain
//! validation, so every value interpolated into a request is first parsed
//! against the exact grammar documented here.

use alloy::primitives::{Address, U256};

/// A lowercase or checksummed `0x` + 40 hex character address, without any
/// whitespace, prefixed slash, or percent-encoding. Returns the parsed
/// address so callers build URLs from its canonical lowercase form.
pub fn parse_address(value: &str) -> Result<Address, String> {
    let body = value
        .strip_prefix("0x")
        .ok_or_else(|| "address must start with 0x".to_string())?;
    if body.len() != 40 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("address must be 0x followed by 40 hex characters".into());
    }
    value
        .parse::<Address>()
        .map_err(|_| "address is not a valid EVM address".into())
}

/// A plain decimal token amount in atoms: digits only, no sign, no
/// exponent, at most 78 digits (uint256), and non-zero.
pub fn parse_amount(value: &str) -> Result<U256, String> {
    if value.is_empty() || value.len() > 78 {
        return Err("amount must be 1..=78 decimal digits".into());
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("amount must be a plain decimal string of atoms".into());
    }
    if value.bytes().all(|b| b == b'0') {
        return Err("amount must be greater than zero".into());
    }
    value
        .parse::<U256>()
        .map_err(|_| "amount does not fit in a uint256".into())
}

/// A `<sellToken>-<buyToken>` pair segment: exactly one `-` joining two
/// address grammar strings.
pub fn parse_pair(value: &str) -> Result<(Address, Address), String> {
    let (sell, buy) = value
        .split_once('-')
        .ok_or_else(|| "pair must be <sellToken>-<buyToken>".to_string())?;
    if sell.contains('-') || buy.contains('-') {
        return Err("pair must contain exactly one '-' separator".into());
    }
    Ok((parse_address(sell)?, parse_address(buy)?))
}

/// A decimal basis-point slippage bound: 1..=5000 (0.01%..=50%).
pub fn parse_slippage_bps(value: u64) -> Result<u64, String> {
    if !(1..=5000).contains(&value) {
        return Err("slippage_bps must be between 1 and 5000".into());
    }
    Ok(value)
}

/// A CoW order UID: `0x` + 112 hex chars (56 bytes: digest ‖ owner ‖ validTo).
pub fn parse_order_uid(value: &str) -> Result<[u8; 56], String> {
    let body = value
        .strip_prefix("0x")
        .ok_or_else(|| "order uid must start with 0x".to_string())?;
    if body.len() != 112 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("order uid must be 0x followed by 112 hex characters".into());
    }
    let bytes = hex::decode(body).map_err(|_| "order uid is not hex".to_string())?;
    let mut uid = [0u8; 56];
    uid.copy_from_slice(&bytes);
    Ok(uid)
}

/// A draft id minted by this Petal: `od-` + 16 lowercase hex chars. The minted
/// ids are hex so they are safe store-key and URL segments by construction;
/// unknown shapes are refused rather than normalized.
pub fn parse_draft_id(value: &str) -> Result<(), String> {
    let body = value
        .strip_prefix("od-")
        .ok_or_else(|| "draft id must start with od-".to_string())?;
    if body.len() != 16 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("draft id must be od- followed by 16 hex characters".into());
    }
    if !body.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        return Err("draft id hex must be lowercase".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAI: &str = "0x6b175474e89094c44da98b954eedeac495271d0f";
    const WETH: &str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";

    #[test]
    fn addresses_accept_lower_and_checksummed_forms() {
        assert!(parse_address(DAI).is_ok());
        assert!(parse_address(WETH).is_ok());
    }

    #[test]
    fn addresses_reject_injection_shapes() {
        for bad in [
            "",
            "0x",
            "0x1234",
            "6b175474e89094c44da98b954eedeac495271d0f",
            "0x6b175474e89094c44da98b954eedeac495271d0f/",
            "0x6b175474e89094c44da98b954eedeac495271d0f?x=1",
            "0x6b175474e89094c44da98b954eedeac495271d0g",
            " 0x6b175474e89094c44da98b954eedeac495271d0f",
            "0X6B175474E89094C44DA98B954EEDEAC495271D0F",
        ] {
            assert!(parse_address(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn amounts_accept_decimal_atoms_only() {
        assert_eq!(parse_amount("1").unwrap(), U256::from(1u64));
        assert_eq!(
            parse_amount("1000000000000000000").unwrap(),
            U256::from(1_000_000_000_000_000_000u64)
        );
        for bad in [
            "", "0", "-1", "1.5", "1e18", "0x10", " 1", "1 ", "1_000",
            "115792089237316195423570985008687907853269984665640564039457584007913129639936",
        ] {
            assert!(parse_amount(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn pairs_split_exactly_once() {
        let (sell, buy) = parse_pair(&format!("{DAI}-{WETH}")).unwrap();
        assert_eq!(format!("{sell:#x}"), DAI);
        assert_eq!(format!("{buy:#x}"), WETH.to_lowercase());
        assert!(parse_pair(DAI).is_err());
        assert!(parse_pair(&format!("{DAI}-{WETH}-0x0")).is_err());
        assert!(parse_pair(&format!("{DAI}_")).is_err());
    }

    #[test]
    fn slippage_bounds_are_enforced() {
        assert!(parse_slippage_bps(1).is_ok());
        assert!(parse_slippage_bps(5000).is_ok());
        assert!(parse_slippage_bps(0).is_err());
        assert!(parse_slippage_bps(5001).is_err());
    }

    #[test]
    fn order_uids_must_be_56_bytes_of_hex() {
        let uid = format!("0x{}", "ab".repeat(56));
        assert!(parse_order_uid(&uid).is_ok());
        assert_eq!(parse_order_uid(&uid).unwrap(), [0xabu8; 56]);
        assert!(parse_order_uid(&format!("0x{}", "ab".repeat(55))).is_err());
        assert!(parse_order_uid("0xzz").is_err());
    }

    #[test]
    fn draft_ids_have_a_closed_grammar() {
        assert!(parse_draft_id("od-0123456789abcdef").is_ok());
        assert!(parse_draft_id("od-0123456789ABCDEF").is_err());
        assert!(parse_draft_id("od-0123").is_err());
        assert!(parse_draft_id("xx-0123456789abcdef").is_err());
        assert!(parse_draft_id("od-../secrets").is_err());
        assert!(parse_draft_id("od-0123456789abcde/1").is_err());
    }
}
