//! The confirm write: build the EIP-712 order from a stored draft, obtain
//! the owner's signature through the Bloom signing host, and post the order
//! to the orderbook. Idempotent: re-confirming a posted order is a no-op, and
//! a pending owner approval is recorded so the exact same write can be
//! retried after approval.
//!
//! The order's UID is fixed by the order, so it is known before anything is
//! posted. The draft is saved as `ambiguous` before the POST, and the next
//! confirm looks that UID up in the orderbook. A lost response, a failed
//! store write or a stopped Machine therefore cannot leave a live order the
//! draft does not know about.
//!
//! The signing request carries the order's full EIP-712 typed data as a
//! [`petal::MessageReview`] envelope ([`order::message_review`]). Bloom
//! refuses an EIP-712 signature without one; with it, Broker rebuilds the
//! preimage, checks it is the payload being signed, and shows the owner
//! every `Order` field.

use alloy::primitives::{Address, Signature, U256};
use petal::sdk::{DispatchResponse, HostStatus, SignOutcome, SdkError};
use petal::{Ctx, SignSelector, WalletAccount};

use crate::cow::draft::{self, ApprovalState, DraftStatus, OrderDraft, SIGNING_INTENT};
use crate::cow::http;
use crate::cow::order::{self, Order};
use crate::cow::params;
use crate::cow::CowNetwork;

/// Every CoW order signature is an EIP-712 digest signed by the owner's EVM
/// key.
const SUITE: petal::CryptoSuite = petal::CryptoSuite::Secp256k1Keccak256Recoverable;

/// How much remaining validity an order needs before it is worth signing.
const MIN_REMAINING_VALIDITY_SECONDS: u64 = 60;

/// Confirm a draft: sign and post. See the module docs for lifecycle.
pub fn confirm_draft(
    ctx: &Ctx,
    network: CowNetwork,
    account: &WalletAccount,
    id: &str,
    body: &[u8],
) -> DispatchResponse {
    if let Err(e) = params::parse_draft_id(id) {
        return petal::error(-3, format!("draft id: {e}"));
    }
    let confirm: ConfirmBody = match serde_json::from_slice(body) {
        Ok(body) => body,
        Err(e) => return petal::error(-3, format!("confirm JSON: {e}")),
    };
    if !confirm.confirm {
        return petal::error(-3, "confirm must be true");
    }
    let key = draft::draft_key(network.api_env(), account.wallet(), account.number(), id);
    let mut stored = match draft::load_draft(&key) {
        Ok(draft) => draft,
        Err(resp) => return resp,
    };
    if stored.network != network.api_env()
        || stored.wallet != account.wallet()
        || stored.account != account.number()
    {
        return petal::error(-4, "draft identity mismatch");
    }
    // Idempotent re-confirm of an already-posted order. An earlier write may
    // have recorded the post without the UID, so fill it in.
    if stored.status == DraftStatus::Posted {
        if stored.uid.is_none()
            && let Ok(uid) = stored.expected_uid(network)
        {
            stored.uid = Some(uid);
            let _ = draft::save_draft(&key, &stored);
        }
        return DispatchResponse::Write;
    }
    stored.attempts += 1;
    // An earlier post whose outcome is unknown is reconciled before anything
    // is signed again: the UID is deterministic, so a found order means the
    // earlier POST landed.
    if stored.status == DraftStatus::Ambiguous {
        match reconcile(&stored, network) {
            Reconciled::Posted(uid) => return settle_posted(&key, &mut stored, uid),
            // The orderbook has no order with this UID, so sign and post the
            // same order again. That cannot create a second order: the UID
            // is fixed by the order, and the orderbook refuses a duplicate.
            Reconciled::NotPosted => {}
            Reconciled::StillUnknown => {
                let _ = draft::save_draft(&key, &stored);
                return petal::error(
                    -4,
                    "previous post outcome still unknown; check the order status route before retrying",
                );
            }
            Reconciled::Failed(resp) => return resp,
        }
    }
    if stored.status == DraftStatus::Rejected {
        return petal::error(-3, "draft was rejected by the orderbook; create a new draft");
    }
    let owner = match stored.owner.parse::<Address>() {
        Ok(address) => address,
        Err(_) => return petal::error(-4, "stored owner address is invalid"),
    };
    let account_address = match account.address_evm() {
        Ok(address) => address,
        Err(resp) => return resp,
    };
    if !account_address.eq_ignore_ascii_case(&stored.owner) {
        return petal::error(-4, "draft owner does not match this account's address");
    }
    let gpv2 = match stored.order.to_gpv2() {
        Ok(order) => order,
        Err(e) => return petal::error(-4, format!("stored order is invalid: {e}")),
    };
    // Freshness: do not sign an order about to lapse.
    let now_ms = match petal::sdk::try_now_ms() {
        Ok(ms) => ms,
        Err(e) => return petal::error(-4, format!("read clock: {}", petal::sdk::describe_error(&e))),
    };
    if now_ms / 1000 + MIN_REMAINING_VALIDITY_SECONDS > u64::from(gpv2.validTo) {
        // An earlier confirm may have posted this order without the draft
        // recording it. Look before telling the agent to trade again.
        if stored.attempts > 1 {
            match reconcile(&stored, network) {
                Reconciled::Posted(uid) => return settle_posted(&key, &mut stored, uid),
                Reconciled::NotPosted => {}
                Reconciled::StillUnknown => {
                    return petal::error(
                        -4,
                        "quote has expired, and the orderbook could not be checked for an earlier post of this order; retry this confirm before creating a new draft",
                    );
                }
                Reconciled::Failed(resp) => return resp,
            }
        }
        return petal::error(-3, "quote has expired; create a new draft for a fresh quote");
    }
    let review = match order::message_review(&gpv2, network) {
        Ok(review) => review,
        Err(e) => return petal::error(-4, format!("order message review: {e}")),
    };
    let preimage = order::signing_preimage(&gpv2, network);
    let payload = petal::PayloadSignItem {
        claimed_hash: alloy::primitives::keccak256(&preimage).0,
        preimage,
    };
    let effects = claim_effects(&stored, network);
    let claim = match petal::petal_use_claim(
        ctx,
        SIGNING_INTENT,
        SUITE,
        std::slice::from_ref(&payload),
        &effects,
        petal::ClaimNonce::Derived,
    ) {
        Ok(claim) => claim,
        Err(resp) => return resp,
    };
    let mut request = match claim.sign_request(account.wallet(), SignSelector::Exact) {
        Ok(request) => request,
        Err(resp) => return resp,
    };
    request.action = Some(review.into_bytes());
    match petal::sdk::sign_payload(&request) {
        Ok(SignOutcome::Signature(raw)) => {
            // Record that a post is about to happen before making it. If the
            // response is lost, the store write after it fails or Machine
            // stops, the next confirm finds an `ambiguous` draft and looks
            // the order up, instead of a draft that looks unposted.
            stored.status = DraftStatus::Ambiguous;
            stored.approval = None;
            stored.last_error = Some("order signed; its post was not recorded".into());
            if let Err(resp) = draft::save_draft(&key, &stored) {
                return resp;
            }
            post_signed_order(&key, &mut stored, network, &gpv2, owner, &raw)
        }
        Ok(SignOutcome::ApprovalPending { action_id, expires_ms }) => {
            let message = format!(
                "owner approval required for this order (request {action_id}); approve it in Bloom, then retry this exact confirm write"
            );
            stored.status = DraftStatus::SigningPending;
            stored.approval = Some(ApprovalState { action_id, expires_ms });
            stored.last_error = None;
            if let Err(resp) = draft::save_draft(&key, &stored) {
                return resp;
            }
            petal::error(-3, message)
        }
        Err(e @ SdkError::Host(HostStatus::Denied)) => {
            // Read the host's reason before saving the draft replaces it. It
            // is returned with -3: Bloom drops the message of a -2, and the
            // agent needs to know whether to wait, retry or start again.
            let reason = petal::sdk::describe_error(&e);
            let lapsed = stored.status == DraftStatus::ApprovalLapsed;
            let denied = denied(stored.approval.as_ref(), lapsed, now_ms, &reason);
            if let Some(status) = denied.status {
                stored.status = status;
            }
            if denied.clear_approval {
                stored.approval = None;
            }
            stored.last_error = Some(denied.message.clone());
            let _ = draft::save_draft(&key, &stored);
            petal::error(-3, denied.message)
        }
        Err(e) => {
            // Read the host's reason before saving the draft replaces it.
            let message = format!("signing failed: {}", petal::sdk::describe_error(&e));
            stored.last_error = Some(message.clone());
            let _ = draft::save_draft(&key, &stored);
            petal::error(-4, message)
        }
    }
}

/// What a denied signature means for the draft, and what to tell the agent.
#[derive(Debug, PartialEq, Eq)]
struct Denied {
    /// The status to move the draft to, when the denial changes it.
    status: Option<DraftStatus>,
    clear_approval: bool,
    message: String,
}

/// Bloom denies a signature for several reasons, and the right next step
/// differs. While a recorded approval is still inside its window, the owner
/// most likely has not approved yet, so the draft stays pending. Once the
/// window has passed, the approval expired or was declined: the draft is
/// marked lapsed and the agent is told to start again, and it is told so
/// again on any later attempt (`lapsed`). `reason` is the host's own
/// explanation.
fn denied(approval: Option<&ApprovalState>, lapsed: bool, now_ms: u64, reason: &str) -> Denied {
    match approval {
        Some(approval) if now_ms < approval.expires_ms => Denied {
            status: None,
            clear_approval: false,
            message: format!(
                "Bloom has not signed this order ({reason}). If the owner has not approved request {} yet, approve it in Bloom, then retry this exact confirm write",
                approval.action_id
            ),
        },
        Some(_) => Denied {
            status: Some(DraftStatus::ApprovalLapsed),
            clear_approval: true,
            message: format!(
                "Bloom refused to sign this order ({reason}). Its owner approval expired or was declined; create a new draft"
            ),
        },
        None if lapsed => Denied {
            status: None,
            clear_approval: false,
            message: format!(
                "Bloom refused to sign this order ({reason}). Its owner approval already expired or was declined; create a new draft"
            ),
        },
        None => Denied {
            status: None,
            clear_approval: false,
            message: format!("Bloom refused to sign this order ({reason})"),
        },
    }
}

/// Record that the orderbook holds this draft's order.
fn settle_posted(key: &str, stored: &mut OrderDraft, uid: String) -> DispatchResponse {
    stored.status = DraftStatus::Posted;
    stored.uid = Some(uid);
    stored.approval = None;
    stored.last_error = None;
    // If this write fails the draft stays `ambiguous`, and the next confirm
    // or status read finds the order by its UID again.
    let _ = draft::save_draft(key, stored);
    DispatchResponse::Write
}

#[derive(Debug, serde::Deserialize)]
struct ConfirmBody {
    confirm: bool,
}

/// The value effects the claim declares for an order: the most that can
/// leave the wallet (sell amount plus fee, in the sell token) and the
/// destination that pulls it (the Vault Relayer). No network fee is declared
/// — CoW fees are taken inside the signed amounts and Bloom currently
/// refuses declared fees.
fn claim_effects(draft: &OrderDraft, network: CowNetwork) -> petal::ClaimEffects {
    let sell = U256::from_str_radix(&draft.order.sell_amount, 10).unwrap_or_default();
    let fee = U256::from_str_radix(&draft.order.fee_amount, 10).unwrap_or_default();
    petal::ClaimEffects::moves_value()
        .debit(
            &petal::ClaimAsset::new(network.bloom_chain(), &draft.order.sell_token),
            &(sell + fee).to_string(),
        )
        .destination(network.bloom_chain(), crate::cow::network::VAULT_RELAYER)
}

enum Reconciled {
    /// The orderbook holds the order under this UID.
    Posted(String),
    NotPosted,
    StillUnknown,
    Failed(DispatchResponse),
}

/// Look the locally computed order UID up in the orderbook.
fn reconcile(draft: &OrderDraft, network: CowNetwork) -> Reconciled {
    let uid = match draft.expected_uid(network) {
        Ok(uid) => uid,
        Err(_) => return Reconciled::StillUnknown,
    };
    match http::get_order(network, &uid) {
        Ok(Some(found)) if found.get("uid").is_some() => Reconciled::Posted(uid),
        Ok(Some(_)) => Reconciled::StillUnknown,
        Ok(None) => Reconciled::NotPosted,
        Err(DispatchResponse::Error { code: -4, .. }) => Reconciled::StillUnknown,
        Err(other) => Reconciled::Failed(other),
    }
}

/// Verify the host signature, build the order creation body, POST it, and
/// record the outcome.
fn post_signed_order(
    key: &str,
    stored: &mut OrderDraft,
    network: CowNetwork,
    gpv2: &Order,
    owner: Address,
    raw_signature: &[u8],
) -> DispatchResponse {
    let digest = order::order_digest(gpv2, network);
    let signature = match Signature::from_raw(raw_signature) {
        Ok(signature) => signature,
        Err(e) => {
            stored.status = DraftStatus::Rejected;
            stored.last_error = Some(format!("host signature malformed: {e}"));
            let _ = draft::save_draft(key, stored);
            return petal::error(-4, "host returned a malformed signature");
        }
    };
    match signature.recover_address_from_prehash(&digest) {
        Ok(recovered) if recovered == owner => {}
        Ok(_recovered) => {
            stored.status = DraftStatus::Rejected;
            stored.last_error = Some("host signature recovered to a different address".into());
            let _ = draft::save_draft(key, stored);
            return petal::error(-4, "host signature does not match the draft owner");
        }
        Err(e) => {
            stored.status = DraftStatus::Rejected;
            stored.last_error = Some(format!("signature recovery failed: {e}"));
            let _ = draft::save_draft(key, stored);
            return petal::error(-4, "could not recover the host signature");
        }
    }
    // Signature bytes never touch the store; the posted order is public on
    // the orderbook, so nothing needs persisting from this point.
    let body = order_creation_body(stored, gpv2, owner, &signature, network);
    let expected_uid = format!("0x{}", hex::encode(order::order_uid(gpv2, owner, network)));
    match http::post_order(network, &body) {
        Ok(http::OrderPost::Created(response)) => {
            let (uid, note) = accepted_uid(response.as_str(), &expected_uid);
            stored.status = DraftStatus::Posted;
            stored.uid = Some(uid.clone());
            stored.approval = None;
            stored.last_error = note;
            match draft::save_draft(key, stored) {
                Ok(()) => DispatchResponse::Write,
                // The order is live. The draft stays `ambiguous`, which the
                // next confirm settles by looking the UID up.
                Err(DispatchResponse::Error { message, .. }) => petal::error(
                    -4,
                    format!("order {uid} was posted, but the draft could not be updated ({message}); retry this confirm to record it"),
                ),
                Err(resp) => resp,
            }
        }
        Ok(http::OrderPost::Refused {
            status,
            error_type,
            message,
        }) => match refusal(status, error_type.as_deref()) {
            Refusal::AlreadyPosted => {
                stored.status = DraftStatus::Posted;
                stored.uid = Some(expected_uid);
                stored.approval = None;
                stored.last_error = None;
                match draft::save_draft(key, stored) {
                    Ok(()) => DispatchResponse::Write,
                    Err(resp) => resp,
                }
            }
            Refusal::RetryLater => {
                // The orderbook did not create the order, so nothing is
                // ambiguous: the draft awaits another confirm, which signs
                // and posts again.
                stored.status = DraftStatus::Quoted;
                stored.last_error = Some(message.clone());
                let _ = draft::save_draft(key, stored);
                petal::error(-4, format!("the orderbook is rate limiting; retry this confirm: {message}"))
            }
            Refusal::Rejected => {
                stored.status = DraftStatus::Rejected;
                stored.last_error = Some(message.clone());
                let _ = draft::save_draft(key, stored);
                petal::error(-3, format!("the orderbook refused the order; create a new draft: {message}"))
            }
        },
        Err(DispatchResponse::Error { code: -4, message }) => {
            // A transport failure or a 5xx leaves the outcome unknown; the
            // next confirm looks the order up by its UID first.
            stored.status = DraftStatus::Ambiguous;
            stored.last_error = Some(message.clone());
            let _ = draft::save_draft(key, stored);
            petal::error(-4, format!("order POST outcome unknown after signing: {message}"))
        }
        Err(resp) => resp,
    }
}

/// The UID to record for a created order, and a note when the orderbook's
/// answer was not the UID this Petal computed. The orderbook is
/// authoritative for the order it accepted, so its UID is kept when it is a
/// well-formed one; the difference is recorded so it is not silent.
fn accepted_uid(returned: Option<&str>, expected: &str) -> (String, Option<String>) {
    match returned {
        Some(uid) if uid.eq_ignore_ascii_case(expected) => (expected.to_string(), None),
        Some(uid) if params::parse_order_uid(uid).is_ok() => (
            uid.to_ascii_lowercase(),
            Some(format!(
                "the orderbook returned order uid {uid}, but this order's computed uid is {expected}"
            )),
        ),
        _ => (expected.to_string(), None),
    }
}

/// What a 4xx answer to an order POST means for the draft. The orderbook did
/// not create the order on that request: `DuplicatedOrder` means an earlier
/// POST of this same order did, and a 429 asks for a later retry.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    AlreadyPosted,
    RetryLater,
    Rejected,
}

fn refusal(status: u16, error_type: Option<&str>) -> Refusal {
    match (status, error_type) {
        (_, Some("DuplicatedOrder")) => Refusal::AlreadyPosted,
        (429, _) => Refusal::RetryLater,
        _ => Refusal::Rejected,
    }
}

/// The `OrderCreation` body, per the orderbook OpenAPI schema. Public so the
/// Sepolia end-to-end example (`route/examples/sepolia_e2e.rs`) submits the
/// byte-identical body this Petal's confirm route builds.
pub fn order_creation_body(
    stored: &OrderDraft,
    gpv2: &Order,
    owner: Address,
    signature: &Signature,
    _network: CowNetwork,
) -> serde_json::Value {
    let _ = stored;
    serde_json::json!({
        "sellToken": format!("{:#x}", gpv2.sellToken),
        "buyToken": format!("{:#x}", gpv2.buyToken),
        "receiver": format!("{:#x}", gpv2.receiver),
        "sellAmount": gpv2.sellAmount.to_string(),
        "buyAmount": gpv2.buyAmount.to_string(),
        "validTo": u64::from(gpv2.validTo),
        "appData": stored.order.app_data_document,
        "appDataHash": stored.order.app_data,
        "feeAmount": gpv2.feeAmount.to_string(),
        "kind": gpv2.kind,
        "partiallyFillable": gpv2.partiallyFillable,
        "sellTokenBalance": gpv2.sellTokenBalance,
        "buyTokenBalance": gpv2.buyTokenBalance,
        "signingScheme": "eip712",
        "signature": order::signature_hex(&signature_as_rsv(signature)).expect("65-byte signature"),
        "from": format!("{owner:#x}"),
        "quoteId": null,
    })
}

/// The raw `r ‖ s ‖ v` bytes with v in {27, 28}, as the API expects.
pub fn signature_as_rsv(signature: &Signature) -> [u8; 65] {
    let bytes = signature.as_bytes();
    let mut rsv = [0u8; 65];
    rsv.copy_from_slice(&bytes[..65]);
    rsv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cow::quote::{QuoteSide, QuotedOrder};

    fn draft_fixture(kind: &str) -> OrderDraft {
        let quoted = QuotedOrder {
            sell_token: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            buy_token: "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2".into(),
            sell_amount: "9999312248".into(),
            buy_amount: "4308666162407131131".into(),
            valid_to: 4_102_444_800,
            fee_amount: "687752".into(),
            kind: kind.into(),
        };
        OrderDraft {
            schema: "bloom.cowswap.order_draft.v1".into(),
            id: "od-0123456789abcdef".into(),
            network: "mainnet".into(),
            chain_id: 1,
            wallet: "kaze".into(),
            account: 0,
            owner: "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266".into(),
            side: QuoteSide::Sell,
            sell_token: quoted.sell_token.clone(),
            buy_token: quoted.buy_token.clone(),
            input_amount: "10000000000".into(),
            slippage_bps: 50,
            order: draft::order_from_quote(&quoted, QuoteSide::Sell, 50).unwrap(),
            quote: quoted,
            status: DraftStatus::Quoted,
            approval: None,
            uid: None,
            last_error: None,
            attempts: 0,
            created_ms: 0,
        }
    }

    #[test]
    fn order_refusals_are_classified() {
        assert_eq!(refusal(400, Some("NonZeroFee")), Refusal::Rejected);
        assert_eq!(refusal(400, Some("InsufficientBalance")), Refusal::Rejected);
        assert_eq!(refusal(400, None), Refusal::Rejected);
        assert_eq!(refusal(403, Some("Forbidden")), Refusal::Rejected);
        assert_eq!(refusal(400, Some("DuplicatedOrder")), Refusal::AlreadyPosted);
        assert_eq!(refusal(429, None), Refusal::RetryLater);
    }

    /// The envelope confirm sends for a stored draft rebuilds the exact
    /// payload confirm asks Bloom to sign, on every network.
    #[test]
    fn stored_draft_review_rebuilds_the_signed_payload() {
        let gpv2 = draft_fixture("sell").order.to_gpv2().unwrap();
        for network in [
            CowNetwork::Mainnet,
            CowNetwork::Gnosis,
            CowNetwork::Arbitrum,
            CowNetwork::Base,
            CowNetwork::Sepolia,
        ] {
            let review: serde_json::Value =
                serde_json::from_slice(order::message_review(&gpv2, network).unwrap().as_bytes()).unwrap();
            let typed: alloy_dyn_abi::TypedData =
                serde_json::from_value(review["typed_data"].clone()).unwrap();
            assert_eq!(
                typed.eip712_signing_hash().unwrap(),
                alloy::primitives::keccak256(order::signing_preimage(&gpv2, network))
            );
            assert_eq!(review["typed_data"]["message"]["sellAmount"], "10000000000");
            assert_eq!(review["typed_data"]["message"]["feeAmount"], "0");
        }
    }

    #[test]
    fn claim_effects_declare_the_full_sell_debit_to_the_relayer() {
        for network in [
            CowNetwork::Mainnet,
            CowNetwork::Gnosis,
            CowNetwork::Arbitrum,
            CowNetwork::Base,
            CowNetwork::Sepolia,
        ] {
            let effects = claim_effects(&draft_fixture("sell"), network);
            let debits = effects.debits();
            assert_eq!(debits.len(), 1, "one debit on {network:?}");
            assert_eq!(debits[0].amount, "10000000000", "sellAmount + feeAmount");
            let destinations = effects.destinations();
            assert_eq!(destinations.len(), 1);
            assert_eq!(destinations[0].chain, network.bloom_chain());
            assert_eq!(destinations[0].destination, crate::cow::network::VAULT_RELAYER);
        }
    }

    #[test]
    fn order_creation_body_matches_the_openapi_shape() {
        let draft = draft_fixture("sell");
        let gpv2 = draft.order.to_gpv2().unwrap();
        let owner: Address = draft.owner.parse().unwrap();
        use alloy::signers::local::PrivateKeySigner;
        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap();
        let signature = tokio_block_sign(&signer, &gpv2);
        let body = order_creation_body(&draft, &gpv2, owner, &signature, CowNetwork::Mainnet);
        for field in [
            "sellToken",
            "buyToken",
            "receiver",
            "sellAmount",
            "buyAmount",
            "validTo",
            "appData",
            "appDataHash",
            "feeAmount",
            "kind",
            "partiallyFillable",
            "sellTokenBalance",
            "buyTokenBalance",
            "signingScheme",
            "signature",
            "from",
        ] {
            assert!(body.get(field).is_some(), "missing {field}");
        }
        assert_eq!(body["signingScheme"], "eip712");
        assert_eq!(body["kind"], "sell");
        assert_eq!(body["from"], "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        let sig = body["signature"].as_str().unwrap();
        assert!(sig.starts_with("0x") && sig.len() == 132, "65 bytes hex: {sig}");
        let v = u8::from_str_radix(&sig[130..], 16).unwrap();
        assert!(v == 27 || v == 28, "v must be 27/28, got {v}");
        assert_eq!(body["appData"], crate::cow::app_data::app_data_document());
    }

    /// Sign the order digest with a test key, proving the full
    /// struct→digest→ECDSA→rsv path recovers the owner (anvil account 0).
    #[test]
    fn signed_order_digest_recovers_the_owner() {
        let draft = draft_fixture("sell");
        let gpv2 = draft.order.to_gpv2().unwrap();
        let digest = order::order_digest(&gpv2, CowNetwork::Mainnet);
        use alloy::signers::local::PrivateKeySigner;
        let signer: PrivateKeySigner =
            "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80".parse().unwrap();
        let signature = tokio_block_sign(&signer, &gpv2);
        let recovered = signature
            .recover_address_from_prehash(&digest)
            .expect("recovery");
        assert_eq!(format!("{recovered:#x}"), "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
        // rsv encoding round-trips through the wire form.
        let rsv = signature_as_rsv(&signature);
        let wire = order::signature_hex(&rsv).unwrap();
        let raw = hex::decode(&wire[2..]).unwrap();
        let back = Signature::from_raw(&raw).unwrap();
        assert_eq!(back, signature);
    }

    /// Minimal inline async executor: alloy's local signer exposes an async
    /// `sign_hash`; tests drive it to completion on a current-thread park.
    fn tokio_block_sign(
        signer: &alloy::signers::local::PrivateKeySigner,
        gpv2: &Order,
    ) -> Signature {
        use alloy::signers::Signer;
        let digest = order::order_digest(gpv2, CowNetwork::Mainnet);
        let mut future = Box::pin(signer.sign_hash(&digest));
        // sign_hash on the local signer completes without I/O; park until it
        // resolves without pulling in an async runtime dev-dependency.
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        loop {
            match future.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(result) => {
                    return result.expect("local signing cannot fail")
                }
                std::task::Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn approval(expires_ms: u64) -> ApprovalState {
        ApprovalState {
            action_id: "act-123".into(),
            expires_ms,
        }
    }

    #[test]
    fn a_denial_inside_the_approval_window_keeps_the_draft_pending() {
        let outcome = denied(Some(&approval(2_000)), false, 1_999, "denied: approval is not active");
        assert_eq!(outcome.status, None);
        assert!(!outcome.clear_approval);
        // The agent learns the reason and which request to approve.
        assert!(outcome.message.contains("approval is not active"), "{}", outcome.message);
        assert!(outcome.message.contains("act-123"), "{}", outcome.message);
        assert!(outcome.message.contains("retry"), "{}", outcome.message);
    }

    #[test]
    fn a_denial_after_the_approval_window_marks_the_draft_lapsed() {
        for now_ms in [2_000, 2_001, u64::MAX] {
            let outcome = denied(Some(&approval(2_000)), false, now_ms, "denied");
            assert_eq!(outcome.status, Some(DraftStatus::ApprovalLapsed));
            assert!(outcome.clear_approval);
            assert!(outcome.message.contains("create a new draft"), "{}", outcome.message);
        }
    }

    #[test]
    fn a_denial_with_no_recorded_approval_reports_the_reason_only() {
        let outcome = denied(None, false, 5, "denied: CEREMONY_RATE_LIMITED retry after 900 ms");
        assert_eq!(outcome.status, None);
        assert!(!outcome.clear_approval);
        assert!(outcome.message.contains("CEREMONY_RATE_LIMITED"), "{}", outcome.message);
        // Nothing here says the draft is dead: a rate limit is temporary.
        assert!(!outcome.message.contains("new draft"), "{}", outcome.message);
    }

    #[test]
    fn a_draft_whose_approval_lapsed_keeps_saying_so_when_denied_again() {
        let outcome = denied(None, true, 5, "denied");
        assert_eq!(outcome.status, None);
        assert!(outcome.message.contains("create a new draft"), "{}", outcome.message);
    }

    #[test]
    fn the_recorded_uid_is_the_computed_one_unless_the_orderbook_returns_another() {
        let expected = format!("0x{}", "ab".repeat(56));
        let other = format!("0x{}", "CD".repeat(56));
        // The usual answer, in any letter case, records the computed UID.
        assert_eq!(accepted_uid(Some(&expected), &expected), (expected.clone(), None));
        assert_eq!(
            accepted_uid(Some(&expected.to_ascii_uppercase().replace("0X", "0x")), &expected),
            (expected.clone(), None)
        );
        // A different well-formed UID is kept, and the difference is noted.
        let (uid, note) = accepted_uid(Some(&other), &expected);
        assert_eq!(uid, other.to_ascii_lowercase());
        assert!(note.unwrap().contains(&expected));
        // A missing or malformed answer falls back to the computed UID.
        assert_eq!(accepted_uid(None, &expected), (expected.clone(), None));
        assert_eq!(accepted_uid(Some("not-a-uid"), &expected), (expected.clone(), None));
    }
}
