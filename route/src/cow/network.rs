//! The networks this Petal serves, as a closed typed enum.
//!
//! Route files resolve the `[network]` path parameter through
//! [`CowNetwork::from_segment`] *before* any URL, store key, or chain name is
//! built from it, so an unknown segment is rejected as invalid input and can
//! never steer a request at another host or chain.

use serde::{Deserialize, Serialize};

/// GPv2Settlement — same address on every supported chain (verified against
/// the CoW Protocol contracts deployment table).
pub const SETTLEMENT_CONTRACT: &str = "0x9008D19f58AAbD9eD0D60971565AA8510560ab41";

/// GPv2VaultRelayer — the spender ERC20 allowances must be granted to.
/// Same address on every supported chain (verified against the deployment
/// table).
pub const VAULT_RELAYER: &str = "0xC92E8bdf79f0507f65a392b0ab4667716BFE0110";

/// Base URL of the CoW Protocol orderbook API.
pub const API_BASE: &str = "https://api.cow.fi";

/// A CoW Protocol deployment this Petal can talk to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CowNetwork {
    Mainnet,
    Gnosis,
    Arbitrum,
    Base,
}

impl CowNetwork {
    /// Resolve a route `[network]` segment. The accepted spellings are the
    /// exact API environment names from the CoW docs, so the segment maps 1:1
    /// onto the URL path below `https://api.cow.fi`.
    pub fn from_segment(segment: &str) -> Option<Self> {
        match segment {
            "mainnet" => Some(Self::Mainnet),
            "xdai" => Some(Self::Gnosis),
            "arbitrum_one" => Some(Self::Arbitrum),
            "base" => Some(Self::Base),
            _ => None,
        }
    }

    /// The `[network]` route segments, in listing order.
    pub const SEGMENTS: [&'static str; 4] = ["mainnet", "xdai", "arbitrum_one", "base"];

    /// The API environment path segment under `api.cow.fi`.
    pub fn api_env(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Gnosis => "xdai",
            Self::Arbitrum => "arbitrum_one",
            Self::Base => "base",
        }
    }

    /// The EIP-155 chain id orders are signed for on this network.
    pub fn chain_id(self) -> u64 {
        match self {
            Self::Mainnet => 1,
            Self::Gnosis => 100,
            Self::Arbitrum => 42161,
            Self::Base => 8453,
        }
    }

    /// Bloom's chain-route name for `chains/<name>/…` reads and `chain_read`.
    pub fn bloom_chain(self) -> &'static str {
        match self {
            Self::Mainnet => "ethereum",
            Self::Gnosis => "gnosis",
            Self::Arbitrum => "arbitrum",
            Self::Base => "base",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_round_trip_and_map_to_api_environments() {
        for segment in CowNetwork::SEGMENTS {
            let network = CowNetwork::from_segment(segment)
                .unwrap_or_else(|| panic!("segment {segment} must resolve"));
            assert_eq!(network.api_env(), segment);
            assert!(network.chain_id() > 0);
            assert!(!network.bloom_chain().is_empty());
        }
    }

    #[test]
    fn unknown_or_malformed_segments_are_rejected() {
        for bad in [
            "",
            "Mainnet",
            "sepolia",
            "xdai/",
            "gnosis-chain",
            "arbitrum-one",
            "ethereum",
            "mainnet ",
            "../mainnet",
        ] {
            assert_eq!(CowNetwork::from_segment(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn contract_addresses_are_checksummed() {
        for address in [SETTLEMENT_CONTRACT, VAULT_RELAYER] {
            assert!(
                address.starts_with("0x")
                    && address.len() == 42
                    && address[2..].chars().any(|c| c.is_ascii_uppercase()),
                "{address} must be a checksummed address"
            );
            assert!(address.parse::<alloy::primitives::Address>().is_ok());
        }
    }
}
