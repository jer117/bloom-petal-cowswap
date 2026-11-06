//! Authorized end-to-end exercise of this Petal's order pipeline on Sepolia
//! (chain 11155111) — Sepolia ONLY; any other chain id is a hard failure.
//!
//! Flow: load the burner key from `COW_BURNER_KEY` (never printed, never
//! stored) → verify the live GPv2Settlement domain separator against the
//! crate's EIP-712 domain → wait for faucet funds (≤45 min) → quote via
//! `POST /sepolia/api/v1/quote` (2% slippage, validTo now+30min, not
//! partially fillable) → exact-amount ERC20 approve to the GPv2VaultRelayer
//! (EIP-1559) → build + sign the GPv2 order with the crate's own EIP-712
//! encoding and order-creation body (the confirm route's exact code path)
//! → `POST /sepolia/api/v1/orders` → poll the order until fulfilled,
//! expired, or 25 minutes elapse. Every refusal prints the full API body;
//! outcomes (open/expired included) are reported honestly, never faked.
//!
//! Run:
//! ```sh
//! COW_BURNER_KEY=0x<64 hex chars> \
//!   cargo run --manifest-path route/Cargo.toml --locked --example sepolia_e2e
//! ```
//! HTTP uses the system `curl` (the lockfile pins no HTTP client crate);
//! all signing, EIP-712 hashing, and transaction encoding happen
//! in-process via the crate and alloy.

use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use alloy::primitives::{Address, Bytes, Signature, TxKind, U256, keccak256};
use alloy::signers::SignerSync;
use alloy::signers::local::PrivateKeySigner;
use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;

use bloom_cowswap_route::cow::confirm;
use bloom_cowswap_route::cow::draft::{self, DraftStatus, OrderDraft};
use bloom_cowswap_route::cow::network::{CowNetwork, VAULT_RELAYER};
use bloom_cowswap_route::cow::order;
use bloom_cowswap_route::cow::quote::{QuoteSide, QuotedOrder};

const NET: CowNetwork = CowNetwork::Sepolia;
const CHAIN_ID: u64 = 11_155_111;

/// Primary first, then fallbacks. drpc is kept (task-specified fallback) even
/// though its free plan currently refuses Sepolia from this host.
const RPCS: [&str; 4] = [
    "https://ethereum-sepolia-rpc.publicnode.com",
    "https://sepolia.drpc.org",
    "https://1rpc.io/sepolia",
    "https://sepolia.gateway.tenderly.co",
];
const API: &str = "https://api.cow.fi";

/// Circle's native USDC on Sepolia (6 decimals).
const USDC: &str = "0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238";
/// WETH9 on Sepolia (18 decimals).
const WETH: &str = "0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14";
/// The funded burner this test is authorized for.
const EXPECTED_OWNER: &str = "0x2dAe23839c196e0D7734fCE67AfC8DaED73AA396";

const SLIPPAGE_BPS: u64 = 200; // 2%
const VALID_TO_SECONDS: u64 = 30 * 60;
const FUND_WAIT: Duration = Duration::from_secs(45 * 60);
const SETTLE_WAIT: Duration = Duration::from_secs(25 * 60);
const ONE_USDC: U256 = U256::from_limbs([1_000_000, 0, 0, 0]);
const WRAP_WEI: U256 = U256::from_limbs([500_000_000_000_000, 0, 0, 0]); // 0.0005 ETH
const ETH_MIN_SELL: U256 = U256::from_limbs([1_000_000_000_000_000, 0, 0, 0]); // 0.001
const ETH_MIN_WRAP: U256 = U256::from_limbs([2_000_000_000_000_000, 0, 0, 0]); // 0.002

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_secs()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis()
}

/// One HTTP request through the system curl; returns (status, body).
fn curl(method: &str, url: &str, body: &str) -> Result<(u16, String), String> {
    let out = Command::new("curl")
        .args([
            "-sS",
            "-m",
            "45",
            "-X",
            method,
            "-H",
            "content-type: application/json",
            "-H",
            "user-agent: bloom-cowswap-sepolia-e2e/1.0",
            "-w",
            "\n%{http_code}",
            "--data-binary",
            body,
            url,
        ])
        .output()
        .map_err(|e| format!("spawn curl: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "curl exit {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (body, code) = text
        .rsplit_once('\n')
        .ok_or_else(|| "curl wrote no status line".to_string())?;
    let code: u16 = code
        .trim()
        .parse()
        .map_err(|_| format!("bad curl status line {code:?}"))?;
    Ok((code, body.trim().to_string()))
}

/// JSON-RPC over the RPC list; first node that answers wins.
fn rpc(method: &str, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let payload =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
            .to_string();
    let mut last = String::new();
    for url in RPCS {
        match curl("POST", url, &payload) {
            Ok((200, body)) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(value) => {
                    if let Some(result) = value.get("result").filter(|r| !r.is_null()) {
                        return Ok(result.clone());
                    }
                    last = format!("{url}: {body}");
                }
                Err(e) => last = format!("{url}: non-JSON ({e})"),
            },
            Ok((code, body)) => last = format!("{url}: http {code} {body}"),
            Err(e) => last = format!("{url}: {e}"),
        }
    }
    Err(format!("rpc {method} failed everywhere: {last}"))
}

/// Like [`rpc`], but a JSON-RPC `result: null` answer is `Ok(None)` — the
/// correct response for "no receipt yet", not an error.
fn rpc_opt(method: &str, params: &serde_json::Value) -> Result<Option<serde_json::Value>, String> {
    let payload =
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
            .to_string();
    let mut last = String::new();
    for url in RPCS {
        match curl("POST", url, &payload) {
            Ok((200, body)) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(value) => {
                    if let Some(error) = value.get("error") {
                        last = format!("{url}: {error}");
                        continue;
                    }
                    let result = value.get("result").cloned().unwrap_or(serde_json::Value::Null);
                    if result.is_null() {
                        return Ok(None);
                    }
                    return Ok(Some(result));
                }
                Err(e) => last = format!("{url}: non-JSON ({e})"),
            },
            Ok((code, body)) => last = format!("{url}: http {code} {body}"),
            Err(e) => last = format!("{url}: {e}"),
        }
    }
    Err(format!("rpc {method} failed everywhere: {last}"))
}

fn rpc_str(method: &str, params: &serde_json::Value) -> Result<String, String> {
    rpc(method, params)?
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("{method} returned a non-string"))
}

/// Decode a 0x-quantity or 0x-padded word into U256.
fn hex_to_u256(value: &str) -> U256 {
    let body = value.strip_prefix("0x").unwrap_or(value);
    if body.is_empty() {
        return U256::ZERO;
    }
    U256::from_str_radix(body, 16).unwrap_or_default()
}

fn eth_call(to: Address, data: &str) -> Result<String, String> {
    rpc_str(
        "eth_call",
        &serde_json::json!([{"to": format!("{to:#x}"), "data": data}, "latest"]),
    )
}

/// ABI-encode a call: selector + left-padded 32-byte words.
fn abi_call(selector: &[u8], words: &[Vec<u8>]) -> String {
    let mut data = String::from("0x");
    data.push_str(&hex::encode(selector));
    for word in words {
        let mut padded = vec![0u8; 32];
        let n = word.len().min(32);
        padded[32 - n..].copy_from_slice(&word[word.len() - n..]);
        data.push_str(&hex::encode(padded));
    }
    data
}

fn selector(signature: &str) -> Vec<u8> {
    keccak256(signature.as_bytes()).as_slice()[..4].to_vec()
}

fn balance_of(token: Address, holder: Address) -> Result<U256, String> {
    let data = abi_call(&selector("balanceOf(address)"), &[holder.as_slice().to_vec()]);
    Ok(hex_to_u256(&eth_call(token, &data)?))
}

fn eth_balance(addr: Address) -> Result<U256, String> {
    Ok(hex_to_u256(&rpc_str(
        "eth_getBalance",
        &serde_json::json!([format!("{addr:#x}"), "latest"]),
    )?))
}

fn allowance(token: Address, owner: Address, spender: Address) -> Result<U256, String> {
    let data = abi_call(
        &selector("allowance(address,address)"),
        &[owner.as_slice().to_vec(), spender.as_slice().to_vec()],
    );
    Ok(hex_to_u256(&eth_call(token, &data)?))
}

fn main() {
    if let Err(fatal) = execute() {
        eprintln!("\n=== E2E ABORTED: {fatal} ===");
        std::process::exit(1);
    }
}

fn execute() -> Result<(), String> {
    println!("=== cowswap petal — authorized Sepolia end-to-end test ===");
    println!(
        "network: {} chain_id={CHAIN_ID} api={API}/sepolia/api/v1",
        NET.api_env()
    );

    // 0. Key material: env only — never printed, never written anywhere.
    let raw = std::env::var("COW_BURNER_KEY")
        .map_err(|_| "COW_BURNER_KEY is not set (0x + 64 hex chars)".to_string())?;
    let body = raw.strip_prefix("0x").unwrap_or(raw.as_str());
    let bytes = hex::decode(body.trim()).map_err(|e| format!("COW_BURNER_KEY not hex: {e}"))?;
    if bytes.len() != 32 {
        return Err(format!("COW_BURNER_KEY must be 32 bytes, got {}", bytes.len()));
    }
    let key = alloy::primitives::B256::from_slice(&bytes);
    let signer = PrivateKeySigner::from_bytes(&key).map_err(|e| format!("bad key: {e}"))?;
    let owner = signer.address();
    println!("owner: {owner:#x}");
    if !format!("{owner:#x}").eq_ignore_ascii_case(EXPECTED_OWNER) {
        return Err(format!(
            "burner address {owner:#x} is not the expected {EXPECTED_OWNER}; refusing"
        ));
    }

    // 1. Sepolia only — hard fail on any other chain.
    let chain_hex = rpc_str("eth_chainId", &serde_json::json!([]))?;
    let chain = u64::from_str_radix(chain_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("chainId: {e}"))?;
    println!("rpc chain id: {chain}");
    if chain != CHAIN_ID {
        return Err(format!("chain id {chain} != {CHAIN_ID}: SEPOLIA ONLY — aborting"));
    }

    // 2. Verify the crate's EIP-712 domain against the live settlement.
    let settlement: Address = bloom_cowswap_route::cow::network::SETTLEMENT_CONTRACT
        .parse()
        .map_err(|_| "settlement constant".to_string())?;
    let live = eth_call(settlement, &abi_call(&selector("domainSeparator()"), &[]))?;
    let derived = order::domain(NET).hash_struct();
    println!("settlement domainSeparator (live):  {live}");
    println!("settlement domainSeparator (crate): {derived:#x}");
    if !derived
        .to_string()
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(live.trim_start_matches("0x"))
    {
        return Err("crate EIP-712 domain does not match the live settlement".into());
    }
    let relayer: Address = VAULT_RELAYER.parse().map_err(|_| "relayer".to_string())?;
    println!("GPv2VaultRelayer (allowance target): {relayer:#x}");

    let usdc: Address = USDC.parse().map_err(|_| "USDC address".to_string())?;
    let weth: Address = WETH.parse().map_err(|_| "WETH address".to_string())?;

    // 3. Balances; decide the path (poll up to 45 min for faucet funds).
    let mut sell_token = usdc;
    let mut buy_token = weth;
    let mut sell_amount = ONE_USDC;
    let mut sell_label = "USDC";
    let deadline = now_ms() + FUND_WAIT.as_millis();
    loop {
        let eth = eth_balance(owner)?;
        let usdc_bal = balance_of(usdc, owner)?;
        let weth_bal = balance_of(weth, owner)?;
        println!(
            "[funds {}] ETH {} | USDC {} | WETH {}",
            now(),
            fmt_units(eth, 18),
            fmt_units(usdc_bal, 6),
            fmt_units(weth_bal, 18)
        );
        if usdc_bal >= ONE_USDC && eth >= ETH_MIN_SELL {
            println!("[path] selling 1 USDC for WETH");
            break;
        }
        if weth_bal >= WRAP_WEI {
            // Resumable: WETH wrapped by an earlier run is sold directly.
            println!("[path] WETH already wrapped ({weth_bal}): selling for USDC");
            sell_token = weth;
            buy_token = usdc;
            sell_amount = WRAP_WEI;
            sell_label = "WETH";
            break;
        }
        if eth >= ETH_MIN_WRAP {
            println!("[path] ETH only: wrap 0.0005 ETH, then sell WETH for USDC");
            wrap_eth(&signer, weth, WRAP_WEI)?;
            sell_token = weth;
            buy_token = usdc;
            sell_amount = WRAP_WEI;
            sell_label = "WETH";
            break;
        }
        if now_ms() > deadline {
            return Err(format!(
                "no usable funds after 45 min: ETH {} | USDC {} | WETH {} — aborting",
                fmt_units(eth, 18),
                fmt_units(usdc_bal, 6),
                fmt_units(weth_bal, 18)
            ));
        }
        std::thread::sleep(Duration::from_secs(60));
    }

    // 4. Quote → slippage-bounded order, via the crate's own projection.
    let quoted = quote_loop(owner, sell_token, buy_token, sell_amount)?;
    let view = draft::order_from_quote(&quoted, QuoteSide::Sell, SLIPPAGE_BPS)
        .map_err(|e| format!("order_from_quote: {e}"))?;
    let draft = OrderDraft {
        schema: "bloom.cowswap.order_draft.v1".into(),
        id: "od-5e9011a000000001".into(),
        network: NET.api_env().into(),
        chain_id: NET.chain_id(),
        wallet: "sepolia-e2e".into(),
        account: 0,
        owner: format!("{owner:#x}"),
        side: QuoteSide::Sell,
        sell_token: format!("{sell_token:#x}"),
        buy_token: format!("{buy_token:#x}"),
        input_amount: view.sell_amount.clone(),
        slippage_bps: SLIPPAGE_BPS,
        quote: quoted,
        order: view.clone(),
        status: DraftStatus::Quoted,
        approval: None,
        uid: None,
        last_error: None,
        attempts: 0,
        created_ms: now() * 1000,
    };
    println!(
        "[order] sell {} {} (fee {}) → buy ≥ {} | validTo {} | partiallyFillable false",
        view.sell_amount,
        sell_label,
        view.fee_amount,
        view.buy_amount,
        view.valid_to
    );

    // 5. Allowance: exact-amount approve to the Vault Relayer (EIP-1559).
    let need = U256::from_str_radix(&view.sell_amount, 10)
        .map_err(|_| "sellAmount".to_string())?
        + U256::from_str_radix(&view.fee_amount, 10).map_err(|_| "feeAmount".to_string())?;
    let have = allowance(sell_token, owner, relayer)?;
    println!("[allowance] current {have} of needed {need}");
    if have < need {
        approve_exact(&signer, sell_token, relayer, need)?;
        let have = allowance(sell_token, owner, relayer)?;
        if have < need {
            return Err(format!("allowance still {have} < {need} after approve"));
        }
        println!("[allowance] granted {have}");
    }

    // 6+7. Sign with the crate's EIP-712 encoding (the confirm route's exact
    // path) and post. The sepolia orderbook requires feeAmount = 0, so a
    // NonZeroFee refusal adapts the order — feeAmount set to zero, re-signed
    // with the same encoding — exactly once.
    let mut order_view = view.clone();
    let uid = {
        let mut result = Err("order never attempted".to_string());
        for attempt in 0..2 {
            if attempt == 1 {
                println!(
                    "[adapt] orderbook refused a non-zero fee; rebuilding the order with feeAmount = 0 and re-signing"
                );
                order_view.fee_amount = "0".into();
            }
            let mut record = draft.clone();
            record.order = order_view.clone();
            match sign_and_post(&signer, &record, &order_view)? {
                PostOutcome::Posted(uid) => {
                    result = Ok(uid);
                    break;
                }
                PostOutcome::NonZeroFee => continue,
                PostOutcome::Refused(message) => {
                    result = Err(message);
                    break;
                }
            }
        }
        result?
    };
    println!("[posted] order uid {uid} (matches locally computed uid)");

    // 8. Poll the orderbook until a terminal state or 25 minutes.
    let deadline = now_ms() + SETTLE_WAIT.as_millis();
    let mut last_status = String::new();
    loop {
        let (code, resp) = curl("GET", &format!("{API}/sepolia/api/v1/orders/{uid}"), "")
            .map_err(|e| format!("order status: {e}"))?;
        if !(200..300).contains(&code) {
            return Err(format!("order status http {code}: {resp}"));
        }
        let value: serde_json::Value =
            serde_json::from_str(&resp).map_err(|e| format!("order status JSON: {e}"))?;
        let status = value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        if status != last_status {
            println!("[order {}] status: {status}", now());
            last_status = status.clone();
        }
        match status.as_str() {
            "fulfilled" => {
                let sold = value
                    .get("executedSellAmount")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let bought = value
                    .get("executedBuyAmount")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                let tx = value
                    .pointer("/executionData/txHash")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                println!("[order] executedSellAmount {sold}, executedBuyAmount {bought}");
                println!("\n=== E2E RESULT: FULFILLED ===");
                println!("order uid  : {uid}");
                println!("settlement : {tx}");
                println!("explorer   : https://sepolia.etherscan.io/tx/{tx}");
                return Ok(());
            }
            "expired" | "cancelled" => {
                println!("\n=== E2E RESULT: {status} (reported honestly) ===");
                println!("order uid: {uid}");
                println!("full order: {resp}");
                return Ok(());
            }
            _ => {}
        }
        if now_ms() > deadline {
            println!("\n=== E2E RESULT: still OPEN after 25 min (reported honestly) ===");
            println!("order uid: {uid}");
            println!("full order: {resp}");
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

/// Wrap `amount` wei by calling WETH9.deposit() with value (EIP-1559).
fn wrap_eth(signer: &PrivateKeySigner, weth: Address, amount: U256) -> Result<(), String> {
    let data = abi_call(&selector("deposit()"), &[]);
    // This Sepolia "Wrapped Ether" deposit() estimates ~134k gas (not WETH9's
    // ~43k); unused EIP-1559 gas is refunded, so the buffer is free.
    send_tx(signer, weth, amount, &data, 230_000, "WETH deposit")?;
    let got = balance_of(weth, signer.address())?;
    println!("[wrap] WETH balance now {}", fmt_units(got, 18));
    if got < amount {
        return Err(format!("wrap: WETH balance {got} < {amount}"));
    }
    Ok(())
}

/// approve(spender, amount) — exact amount, EIP-1559, ~200k gas.
fn approve_exact(
    signer: &PrivateKeySigner,
    token: Address,
    spender: Address,
    amount: U256,
) -> Result<(), String> {
    let data = abi_call(
        &selector("approve(address,uint256)"),
        &[spender.as_slice().to_vec(), amount.to_be_bytes::<32>().to_vec()],
    );
    println!("[approve] {token:#x}.approve({spender:#x}, {amount})");
    send_tx(signer, token, U256::ZERO, &data, 200_000, "ERC20 approve")?;
    Ok(())
}

/// Sign + send one EIP-1559 transaction and wait for its receipt.
fn send_tx(
    signer: &PrivateKeySigner,
    to: Address,
    value: U256,
    data: &str,
    gas: u64,
    what: &str,
) -> Result<String, String> {
    let owner = signer.address();
    let nonce_hex = rpc_str(
        "eth_getTransactionCount",
        &serde_json::json!([format!("{owner:#x}"), "pending"]),
    )?;
    let nonce = u64::from_str_radix(nonce_hex.trim_start_matches("0x"), 16)
        .map_err(|e| format!("nonce: {e}"))?;
    let gas_price = hex_to_u256(&rpc_str("eth_gasPrice", &serde_json::json!([]))?);
    let priority = (gas_price / U256::from(10u64)).max(U256::from(1_000_000u64));
    let max_fee = gas_price + priority;
    let priority_u: u128 = u128::try_from(priority).map_err(|_| "priority fee overflow")?;
    let max_fee_u: u128 = u128::try_from(max_fee).map_err(|_| "max fee overflow")?;
    println!("[tx:{what}] nonce={nonce} gas={gas} maxFee={max_fee_u} priority={priority_u}");
    let input = Bytes::copy_from_slice(
        &hex::decode(data.trim_start_matches("0x")).map_err(|e| format!("calldata: {e}"))?,
    );
    let tx = TxEip1559 {
        chain_id: CHAIN_ID,
        nonce,
        gas_limit: gas,
        max_fee_per_gas: max_fee_u,
        max_priority_fee_per_gas: priority_u,
        to: TxKind::Call(to),
        value,
        access_list: Default::default(),
        input,
    };
    let sighash = tx.signature_hash();
    let signature = signer
        .sign_hash_sync(&sighash)
        .map_err(|e| format!("sign tx: {e}"))?;
    let signed = tx.into_signed(signature);
    let envelope: TxEnvelope = signed.into();
    let mut raw = Vec::new();
    envelope.encode_2718(&mut raw);
    let expected_hash = keccak256(&raw);
    let tx_hash = rpc_str(
        "eth_sendRawTransaction",
        &serde_json::json!([format!("0x{}", hex::encode(&raw))]),
    )
    .map_err(|e| format!("{what}: broadcast: {e}"))?;
    println!("[tx:{what}] hash {tx_hash}");
    if !tx_hash
        .trim_start_matches("0x")
        .eq_ignore_ascii_case(&hex::encode(expected_hash.as_slice()))
    {
        return Err(format!(
            "{what}: node returned {tx_hash}, expected {expected_hash:#x}"
        ));
    }
    wait_receipt(&tx_hash, what)?;
    Ok(tx_hash)
}

fn wait_receipt(tx_hash: &str, what: &str) -> Result<(), String> {
    let deadline = now_ms() + Duration::from_secs(180).as_millis();
    loop {
        if let Some(receipt) = rpc_opt("eth_getTransactionReceipt", &serde_json::json!([tx_hash]))? {
            let status = receipt
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            let block = receipt
                .get("blockNumber")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            let gas_used = receipt
                .get("gasUsed")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            println!("[tx:{what}] receipt status={status} block={block} gasUsed={gas_used}");
            if status != "0x1" {
                return Err(format!("{what}: transaction reverted in block {block}"));
            }
            return Ok(());
        }
        if now_ms() > deadline {
            return Err(format!("{what}: no receipt after 3 min (tx {tx_hash})"));
        }
        std::thread::sleep(Duration::from_secs(6));
    }
}

/// Quote with bounded retries; prints every API refusal in full. On
/// "amount too low"-style rejections, raises the sell amount (×10, capped by
/// the sell balance) — an adaptation within protocol rules.
fn quote_loop(
    owner: Address,
    sell_token: Address,
    buy_token: Address,
    amount: U256,
) -> Result<QuotedOrder, String> {
    let mut amount = amount;
    for attempt in 1..=4u32 {
        let body = serde_json::json!({
            "sellToken": format!("{sell_token:#x}"),
            "buyToken": format!("{buy_token:#x}"),
            "from": format!("{owner:#x}"),
            "kind": "sell",
            "sellAmountBeforeFee": amount.to_string(),
            "validTo": now() + VALID_TO_SECONDS,
            "appData": bloom_cowswap_route::cow::app_data::app_data_document(),
            "priceQuality": "verified",
        });
        println!("[quote] attempt {attempt}: sell {amount} of {sell_token:#x}");
        let (code, resp) = curl("POST", &format!("{API}/sepolia/api/v1/quote"), &body.to_string())?;
        if (200..300).contains(&code) {
            let value: serde_json::Value =
                serde_json::from_str(&resp).map_err(|e| format!("quote JSON: {e}"))?;
            let quote = value.get("quote").cloned().unwrap_or(serde_json::Value::Null);
            let field = |name: &str| {
                quote
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("quote missing {name}: {resp}"))
            };
            let quoted = QuotedOrder {
                sell_token: field("sellToken")?,
                buy_token: field("buyToken")?,
                sell_amount: field("sellAmount")?,
                buy_amount: field("buyAmount")?,
                valid_to: quote
                    .get("validTo")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| format!("quote missing validTo: {resp}"))?,
                fee_amount: field("feeAmount")?,
                kind: field("kind")?,
            };
            println!(
                "[quote] sellAmount {} feeAmount {} buyAmount {} validTo {} (verified {:?})",
                quoted.sell_amount,
                quoted.fee_amount,
                quoted.buy_amount,
                quoted.valid_to,
                value.get("verified")
            );
            if quoted.kind != "sell" {
                return Err(format!("quote kind {} is not sell", quoted.kind));
            }
            return Ok(quoted);
        }
        println!("[quote] REFUSED http {code} full body: {resp}");
        let lower = resp.to_lowercase();
        let too_small = lower.contains("amount too low")
            || lower.contains("minorder")
            || lower.contains("below the minimum")
            || lower.contains("order amount");
        if too_small && attempt < 4 {
            let balance = balance_of(sell_token, owner)?;
            let raised = amount.saturating_mul(U256::from(10u64));
            if raised <= balance {
                println!("[quote] adapting: raising sell amount to {raised}");
                amount = raised;
                continue;
            }
            return Err(format!(
                "order too small and balance {balance} cannot cover {raised}"
            ));
        }
        let thin = lower.contains("liquidity") || lower.contains("no market");
        if thin && attempt < 4 {
            println!("[quote] no liquidity yet; retrying in 30s");
            std::thread::sleep(Duration::from_secs(30));
            continue;
        }
        return Err(format!("quote refused (http {code}): {resp}"));
    }
    Err("quote attempts exhausted".into())
}

/// Build the EIP-712 signature with the crate's encoding and POST the order.
fn sign_and_post(
    signer: &PrivateKeySigner,
    record: &OrderDraft,
    view: &draft::SignedOrderView,
) -> Result<PostOutcome, String> {
    let owner = signer.address();
    let gpv2 = view.to_gpv2().map_err(|e| format!("order invalid: {e}"))?;
    let preimage = order::signing_preimage(&gpv2, NET);
    let digest = order::order_digest(&gpv2, NET);
    if keccak256(&preimage) != digest {
        return Err("preimage does not hash to the digest".into());
    }
    println!("[sign] order digest {digest:#x} (feeAmount {})", view.fee_amount);
    let signature: Signature = signer
        .sign_hash_sync(&digest)
        .map_err(|e| format!("sign: {e}"))?;
    let recovered = signature
        .recover_address_from_prehash(&digest)
        .map_err(|e| format!("recover: {e}"))?;
    if recovered != owner {
        return Err(format!("signature recovered {recovered:#x}, not the owner"));
    }
    let expected_uid = format!("0x{}", hex::encode(order::order_uid(&gpv2, owner, NET)));
    let body = confirm::order_creation_body(record, &gpv2, owner, &signature, NET);
    Ok(post_order_loop(&body, &expected_uid))
}

/// POST the signed order; prints the full body on any refusal and retries
/// only for transient causes.
enum PostOutcome {
    Posted(String),
    /// The sepolia orderbook requires zero-fee orders.
    NonZeroFee,
    Refused(String),
}

fn post_order_loop(body: &serde_json::Value, expected_uid: &str) -> PostOutcome {
    for attempt in 1..=3u32 {
        println!("[post] attempt {attempt}");
        let (code, resp) = match curl("POST", &format!("{API}/sepolia/api/v1/orders"), &body.to_string()) {
            Ok(pair) => pair,
            Err(e) => return PostOutcome::Refused(format!("order POST transport: {e}")),
        };
        if (200..300).contains(&code) {
            let uid = match serde_json::from_str::<String>(&resp) {
                Ok(uid) => uid,
                Err(_) => {
                    return PostOutcome::Refused(format!("order POST returned non-uid: {resp}"))
                }
            };
            if uid != expected_uid {
                return PostOutcome::Refused(format!(
                    "orderbook returned uid {uid} but locally computed {expected_uid}"
                ));
            }
            return PostOutcome::Posted(uid);
        }
        println!("[post] REFUSED http {code} full body: {resp}");
        if resp.contains("NonZeroFee") {
            return PostOutcome::NonZeroFee;
        }
        let retryable = resp.to_lowercase().contains("try again")
            || resp.to_lowercase().contains("timeout");
        if retryable && attempt < 3 {
            std::thread::sleep(Duration::from_secs(20));
            continue;
        }
        return PostOutcome::Refused(format!("order POST refused (http {code}): {resp}"));
    }
    PostOutcome::Refused("order POST attempts exhausted".into())
}

fn fmt_units(raw: U256, decimals: u32) -> String {
    let base = U256::from(10u64).pow(U256::from(decimals));
    let whole = raw / base;
    let frac = raw % base;
    if frac.is_zero() {
        return format!("{whole}");
    }
    let mut digits = frac.to_string();
    while digits.len() < decimals as usize {
        digits.insert(0, '0');
    }
    let trimmed = digits.trim_end_matches('0');
    format!("{whole}.{trimmed}")
}
