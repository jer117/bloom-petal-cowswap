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

/// The `bloom.message.review.v1` envelope Bloom requires beside any EIP-712
/// signature: the complete typed data for `order` on `network`. Broker
/// rebuilds the signing preimage from it, refuses the approval unless that
/// preimage is the exact payload being signed, and shows the owner every
/// `Order` field. The bytes depend only on the order and the network, so a
/// retried confirm presents the same envelope Machine bound to the approval.
pub fn message_review(order: &Order, network: CowNetwork) -> Vec<u8> {
    let field = |name: &str, ty: &str| serde_json::json!({ "name": name, "type": ty });
    let domain = domain(network);
    let envelope = serde_json::json!({
        "schema": "bloom.message.review.v1",
        "kind": "eip712",
        "typed_data": {
            "types": {
                "EIP712Domain": [
                    field("name", "string"),
                    field("version", "string"),
                    field("chainId", "uint256"),
                    field("verifyingContract", "address"),
                ],
                "Order": [
                    field("sellToken", "address"),
                    field("buyToken", "address"),
                    field("receiver", "address"),
                    field("sellAmount", "uint256"),
                    field("buyAmount", "uint256"),
                    field("validTo", "uint32"),
                    field("appData", "bytes32"),
                    field("feeAmount", "uint256"),
                    field("kind", "string"),
                    field("partiallyFillable", "bool"),
                    field("sellTokenBalance", "string"),
                    field("buyTokenBalance", "string"),
                ],
            },
            "primaryType": "Order",
            "domain": {
                "name": domain.name.as_deref(),
                "version": domain.version.as_deref(),
                "chainId": network.chain_id(),
                "verifyingContract": domain.verifying_contract.map(|address| format!("{address:#x}")),
            },
            "message": {
                "sellToken": format!("{:#x}", order.sellToken),
                "buyToken": format!("{:#x}", order.buyToken),
                "receiver": format!("{:#x}", order.receiver),
                "sellAmount": order.sellAmount.to_string(),
                "buyAmount": order.buyAmount.to_string(),
                "validTo": order.validTo,
                "appData": format!("{:#x}", order.appData),
                "feeAmount": order.feeAmount.to_string(),
                "kind": order.kind,
                "partiallyFillable": order.partiallyFillable,
                "sellTokenBalance": order.sellTokenBalance,
                "buyTokenBalance": order.buyTokenBalance,
            },
        },
    });
    serde_json::to_vec(&envelope).expect("a JSON value always serializes")
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
            CowNetwork::Sepolia,
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
        // Sepolia (chain 11155111): the domain separator this digest embeds,
        // 0xdaee378bd0eb30ddf479272accf91761e697bc00e067a268f95f1d2732ed230b,
        // was verified against the live GPv2Settlement.domainSeparator()
        // eth_call on Sepolia (2026-11-04), where GPv2Settlement and
        // GPv2VaultRelayer deploy at the same addresses as mainnet.
        assert_eq!(
            order_digest(&order, CowNetwork::Sepolia).to_string(),
            "0x141742acbc5552b381ca1fb85c77e0417d620c5325b12ace97453e9c0108b4fa"
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

    const ALL_NETWORKS: [CowNetwork; 5] = [
        CowNetwork::Mainnet,
        CowNetwork::Gnosis,
        CowNetwork::Arbitrum,
        CowNetwork::Base,
        CowNetwork::Sepolia,
    ];

    fn review_json(order: &Order, network: CowNetwork) -> serde_json::Value {
        serde_json::from_slice(&message_review(order, network)).unwrap()
    }

    /// Broker rebuilds the preimage from the envelope's typed data and
    /// refuses the approval unless it hashes to the payload being signed.
    /// Rebuild it the same way, with alloy's EIP-712 typed-data reader.
    #[test]
    fn message_review_rebuilds_the_signed_digest() {
        let buy = Order {
            kind: "buy".into(),
            partiallyFillable: true,
            receiver: Address::from_str("0x00000000000000000000000000000000000000ff").unwrap(),
            feeAmount: U256::from(7u64),
            sellAmount: U256::MAX,
            ..example_order()
        };
        for order in [example_order(), buy] {
            for network in ALL_NETWORKS {
                let review = review_json(&order, network);
                let typed: alloy_dyn_abi::TypedData =
                    serde_json::from_value(review["typed_data"].clone()).unwrap();
                assert_eq!(
                    typed.eip712_signing_hash().unwrap(),
                    order_digest(&order, network),
                    "{network:?}"
                );
                let mut preimage = vec![0x19, 0x01];
                preimage.extend_from_slice(typed.domain.separator().as_slice());
                preimage.extend_from_slice(typed.hash_struct().unwrap().as_slice());
                assert_eq!(preimage, signing_preimage(&order, network), "{network:?}");
            }
        }
    }

    /// Machine binds the envelope bytes into the approval, so a retry must
    /// send exactly the same bytes.
    #[test]
    fn message_review_bytes_are_stable() {
        let order = example_order();
        assert_eq!(
            message_review(&order, CowNetwork::Mainnet),
            message_review(&order.clone(), CowNetwork::Mainnet)
        );
        assert_ne!(
            message_review(&order, CowNetwork::Mainnet),
            message_review(&order, CowNetwork::Gnosis)
        );
    }

    fn keys(value: &serde_json::Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        keys
    }

    fn assert_broker_bounds(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    assert!(
                        !key.is_empty()
                            && key.len() <= 128
                            && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b':'),
                        "property name {key:?}"
                    );
                    assert_broker_bounds(value);
                }
            }
            serde_json::Value::Array(values) => values.iter().for_each(assert_broker_bounds),
            serde_json::Value::String(text) => assert!(text.len() <= 8192),
            _ => {}
        }
    }

    /// The shape Broker's `bloom.message.review.v1` reader accepts: no member
    /// it does not expect, anywhere, because it refuses unsigned extras.
    #[test]
    fn message_review_matches_the_broker_envelope_shape() {
        for network in ALL_NETWORKS {
            let review = review_json(&example_order(), network);
            assert_broker_bounds(&review);
            assert_eq!(keys(&review), ["kind", "schema", "typed_data"]);
            assert_eq!(review["schema"], "bloom.message.review.v1");
            assert_eq!(review["kind"], "eip712");
            let typed = &review["typed_data"];
            assert_eq!(keys(typed), ["domain", "message", "primaryType", "types"]);
            assert_eq!(typed["primaryType"], "Order");
            for key in keys(&typed["domain"]) {
                assert!(
                    ["name", "version", "chainId", "verifyingContract", "salt"].contains(&key),
                    "domain member {key}"
                );
            }
            assert_eq!(typed["domain"]["chainId"], network.chain_id());
            let types = typed["types"].as_object().unwrap();
            for fields in types.values() {
                for field in fields.as_array().unwrap() {
                    assert_eq!(keys(field), ["name", "type"]);
                }
            }
            let order_fields: Vec<&str> = types["Order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|field| field["name"].as_str().unwrap())
                .collect();
            let mut sorted = order_fields.clone();
            sorted.sort_unstable();
            assert_eq!(keys(&typed["message"]), sorted, "message carries exactly the Order fields");
            // The struct's own EIP-712 type string fixes the field order.
            let declared: Vec<String> = types["Order"]
                .as_array()
                .unwrap()
                .iter()
                .map(|field| format!("{} {}", field["type"].as_str().unwrap(), field["name"].as_str().unwrap()))
                .collect();
            assert_eq!(
                format!("Order({})", declared.join(",")),
                Order::eip712_root_type()
            );
        }
    }
}
