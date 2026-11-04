//! The GPv2 order struct and its EIP-712 encoding, exactly as the CoW
//! Protocol settlement contract verifies it.
//!
//! References: CoW Protocol docs "Signing schemes" (domain `Gnosis Protocol`
//! / `v2` / chainId / GPv2Settlement verifying contract; order digest =
//! keccak256(0x1901 ‖ domainSeparator ‖ hashStruct(order))) and the
//! `GPv2Order` type in `cowprotocol/contracts`.

use std::borrow::Cow;

use alloy::dyn_abi::Eip712Domain;
use alloy::primitives::{Address, B256, U256, keccak256};
use alloy::sol;
use alloy::sol_types::SolStruct;

use crate::cow::CowNetwork;

sol! {
    /// The GPv2 order as it is EIP-712 hashed. The struct name MUST stay
    /// `Order`: the EIP-712 typehash is derived from the name, and the
    /// settlement contract hashes `Order(address sellToken,…)`. Field order
    /// is the contract's canonical struct order; do not reorder either.
    #[allow(missing_docs)]
    struct Order {
        address sellToken;
        address buyToken;
        address receiver;
        uint256 sellAmount;
        uint256 buyAmount;
        uint32 validTo;
        bytes32 appData;
        uint256 feeAmount;
        string kind;
        bool partiallyFillable;
        string sellTokenBalance;
        string buyTokenBalance;
    }
}

/// The EIP-712 domain for GPv2 orders on `network`. The domain name keeps its
/// historical spelling `Gnosis Protocol` — the docs note the settlement
/// contract was never redeployed to rename it.
pub fn domain(network: CowNetwork) -> Eip712Domain {
    Eip712Domain {
        name: Some(Cow::Borrowed("Gnosis Protocol")),
        version: Some(Cow::Borrowed("v2")),
        chain_id: Some(U256::from(network.chain_id())),
        verifying_contract: Some(
            crate::cow::network::SETTLEMENT_CONTRACT
                .parse::<Address>()
                .expect("settlement address constant is valid"),
        ),
        ..Eip712Domain::default()
    }
}

/// `0x1901 ‖ domainSeparator ‖ hashStruct(order)` — the payload the owner
/// signs.
pub fn signing_preimage(order: &Order, network: CowNetwork) -> Vec<u8> {
    let mut preimage = Vec::with_capacity(66);
    preimage.extend_from_slice(&[0x19, 0x01]);
    preimage.extend_from_slice(domain(network).hash_struct().as_slice());
    preimage.extend_from_slice(order.eip712_hash_struct().as_slice());
    preimage
}

/// The EIP-712 order digest: keccak256 of the signing preimage. This is what
/// the ECDSA signature is made over and the first 32 bytes of the order UID.
pub fn order_digest(order: &Order, network: CowNetwork) -> B256 {
    keccak256(signing_preimage(order, network))
}

/// The 56-byte order UID: `digest ‖ owner ‖ validTo`, as computed by
/// `GPv2Settlement`'s `computedOrderUid`. Used to reconcile an ambiguous
/// order POST without trusting a retried submission.
pub fn order_uid(order: &Order, owner: Address, network: CowNetwork) -> [u8; 56] {
    let mut uid = [0u8; 56];
    uid[..32].copy_from_slice(order_digest(order, network).as_slice());
    uid[32..52].copy_from_slice(owner.as_slice());
    uid[52..56].copy_from_slice(&order.validTo.to_be_bytes());
    uid
}

/// Wrap a raw 65-byte `r ‖ s ‖ v` host signature as the hex string the
/// orderbook API expects (`EcdsaSignature`).
pub fn signature_hex(raw: &[u8]) -> Result<String, String> {
    if raw.len() != 65 {
        return Err(format!("signature must be 65 bytes, got {}", raw.len()));
    }
    Ok(format!("0x{}", hex::encode(raw)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    pub(crate) fn example_order() -> Order {
        Order {
            sellToken: Address::from_str("0x6b175474e89094c44da98b954eedeac495271d0f").unwrap(),
            buyToken: Address::from_str("0xdef1ca1fb7fbcdc777520aa7f396b4e015f497ab").unwrap(),
            receiver: Address::ZERO,
            sellAmount: U256::from(10_000_000u64),
            buyAmount: U256::from(10_000u64),
            validTo: 1_704_067_200,
            appData: B256::from_str(
                "0xc85ef7d79691fe79573b1a7064c19c1a9819ebdbd1faaab1a8ec92344438aaf4",
            )
            .unwrap(),
            feeAmount: U256::ZERO,
            kind: "sell".into(),
            partiallyFillable: false,
            sellTokenBalance: "erc20".into(),
            buyTokenBalance: "erc20".into(),
        }
    }

    /// The digest is keccak256 of the preimage, and both change with the
    /// chain id (replay protection).
    #[test]
    fn digest_is_keccak_of_preimage_and_chain_bound() {
        let order = example_order();
        for network in [
            CowNetwork::Mainnet,
            CowNetwork::Gnosis,
            CowNetwork::Arbitrum,
            CowNetwork::Base,
        ] {
            let preimage = signing_preimage(&order, network);
            assert_eq!(order_digest(&order, network), keccak256(&preimage));
            assert_eq!(preimage.len(), 66);
        }
        assert_ne!(
            order_digest(&order, CowNetwork::Mainnet),
            order_digest(&order, CowNetwork::Gnosis)
        );
    }

    /// Digest changes when any order field changes — the struct encoding
    /// covers every field.
    #[test]
    fn digest_binds_every_field() {
        let base = order_digest(&example_order(), CowNetwork::Mainnet);
        for mutated in [
            Order {
                sellAmount: U256::from(10_000_001u64),
                ..example_order()
            },
            Order {
                kind: "buy".into(),
                ..example_order()
            },
            Order {
                partiallyFillable: true,
                ..example_order()
            },
            Order {
                sellTokenBalance: "external".into(),
                ..example_order()
            },
            Order {
                appData: B256::ZERO,
                ..example_order()
            },
            Order {
                validTo: 1_704_067_201,
                ..example_order()
            },
        ] {
            assert_ne!(base, order_digest(&mutated, CowNetwork::Mainnet));
        }
    }

    /// Cross-checked three ways while writing this Petal (see DESIGN.md
    /// "Order encoding verification"): (1) an independent pure-Python EIP-712
    /// implementation using a JSON-RPC node's keccak; (2) the live
    /// GPv2Settlement.domainSeparator() on mainnet, which matches the
    /// domain separator this code derives; (3) the canonical GPv2Order
    /// TYPE_HASH in cowprotocol/contracts, which matches the typehash alloy
    /// derives from the struct below.
    #[test]
    fn digest_matches_independent_cross_check_vector() {
        let order = example_order();
        assert_eq!(
            order_digest(&order, CowNetwork::Mainnet).to_string(),
            "0x5a517609d176fc19edf41688e647d167c456e3f461ea5caca43057153857ac20"
        );
        assert_eq!(
            order_digest(&order, CowNetwork::Gnosis).to_string(),
            "0xf2dc3784643f0f27fb5382642f2a37ab1e99a978b08c8b70107bd3a37b07a106"
        );
        assert_eq!(
            order_digest(&order, CowNetwork::Arbitrum).to_string(),
            "0xa01c0c8075bb3fdc5a599cf0bed4a87dc11af97f87215f03ecf52b41aec092f5"
        );
        assert_eq!(
            order_digest(&order, CowNetwork::Base).to_string(),
            "0x3dd746abcffccc59486df3a9e07d1fe7a6af1ea993fad5a18f315f9d02252b1d"
        );
    }

    /// The UID layout is digest ‖ owner ‖ validTo(BE u32), matching the
    /// orderbook API's UID documentation.
    #[test]
    fn uid_layout_matches_documented_shape() {
        let order = example_order();
        let owner = Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();
        let uid = order_uid(&order, owner, CowNetwork::Mainnet);
        let digest = order_digest(&order, CowNetwork::Mainnet);
        assert_eq!(uid.len(), 56);
        assert_eq!(&uid[..32], digest.as_slice());
        assert_eq!(&uid[32..52], owner.as_slice());
        assert_eq!(uid[52..], 1_704_067_200u32.to_be_bytes());
        let hex_uid = format!("0x{}", hex::encode(uid));
        assert_eq!(hex_uid.len(), 2 + 112);
    }

    #[test]
    fn signature_hex_wraps_raw_bytes() {
        assert_eq!(
            signature_hex(&[0u8; 65]).unwrap(),
            format!("0x{}", "00".repeat(65))
        );
        assert!(signature_hex(&[0u8; 64]).is_err());
    }
}
