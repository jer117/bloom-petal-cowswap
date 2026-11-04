//! ERC20 reads over Bloom's chain host: the allowance and balance checks the
//! CoW order flow needs.
//!
//! The orderbook API has no allowance endpoint (GET /api/v1/allowance/* is
//! gone from the current API), so this Petal reads `allowance(owner,
//! GPv2VaultRelayer)` and `balanceOf(owner)` straight from the token
//! contract with `eth_call`.

use alloy::primitives::{Address, U256};
use alloy::sol;
use alloy::sol_types::SolCall;
use petal::sdk::DispatchResponse;

use crate::cow::CowNetwork;

sol! {
    interface Erc20Reads {
        function balanceOf(address holder) external view returns (uint256);
        function allowance(address owner, address spender) external view returns (uint256);
    }
}

/// Verify Bloom's chain route for `network` really is the chain this Petal
/// signs for, using the VFS chain projection, before any chain call or
/// signature uses it.
pub fn verify_chain_route(network: CowNetwork) -> Result<(), DispatchResponse> {
    let path = format!("chains/{}/chain_id", network.bloom_chain());
    let bytes = petal::sdk::vfs_read(&path, 64)
        .map_err(|e| petal::error(-4, format!("read {path}: {}", e.message())))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| petal::error(-4, format!("{path} is not UTF-8")))?;
    let live: u64 = text
        .trim()
        .parse()
        .map_err(|_| petal::error(-4, format!("{path} is not a decimal chain id")))?;
    if live != network.chain_id() {
        return Err(petal::error(
            -4,
            format!(
                "Bloom chain route {} has chain id {live}, expected {}",
                network.bloom_chain(),
                network.chain_id()
            ),
        ));
    }
    Ok(())
}

fn eth_call(network: CowNetwork, to: Address, data: Vec<u8>, what: &str) -> Result<Vec<u8>, DispatchResponse> {
    let params = serde_json::json!([
        format!("{to:#x}"),
        format!("0x{}", hex::encode(&data)),
        "latest"
    ])
    .to_string();
    let result = petal::sdk::chain_read(network.bloom_chain(), "eth_call", &params)
        .map_err(|e| petal::error(-4, format!("{what}: {}", e.message())))?;
    let result: String = serde_json::from_str(&result)
        .map_err(|_| petal::error(-4, format!("{what} returned non-string JSON")))?;
    let body = result
        .strip_prefix("0x")
        .ok_or_else(|| petal::error(-4, format!("{what} result is not hex")))?;
    hex::decode(body).map_err(|_| petal::error(-4, format!("{what} result is not hex")))
}

/// `token.allowance(owner, spender)`.
pub fn read_allowance(
    network: CowNetwork,
    token: Address,
    owner: Address,
    spender: Address,
) -> Result<U256, DispatchResponse> {
    let call = Erc20Reads::allowanceCall { owner, spender }.abi_encode();
    let data = eth_call(network, token, call, "ERC20 allowance")?;
    Erc20Reads::allowanceCall::abi_decode_returns(&data)
        .map_err(|e| petal::error(-4, format!("ERC20 allowance decode: {e}")))
}

/// `token.balanceOf(holder)`.
pub fn read_balance(
    network: CowNetwork,
    token: Address,
    holder: Address,
) -> Result<U256, DispatchResponse> {
    let call = Erc20Reads::balanceOfCall { holder }.abi_encode();
    let data = eth_call(network, token, call, "ERC20 balanceOf")?;
    Erc20Reads::balanceOfCall::abi_decode_returns(&data)
        .map_err(|e| petal::error(-4, format!("ERC20 balanceOf decode: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erc20_calldata_matches_the_canonical_selectors() {
        let owner = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266".parse::<Address>().unwrap();
        let spender = "0xc92e8bdf79f0507f65a392b0ab4667716bfe0110".parse::<Address>().unwrap();
        // allowance(address,address) selector is 0xdd62ed3e.
        let call = Erc20Reads::allowanceCall { owner, spender }.abi_encode();
        assert_eq!(&call[..4], &[0xdd, 0x62, 0xed, 0x3e]);
        // balanceOf(address) selector is 0x70a08231.
        let call = Erc20Reads::balanceOfCall { holder: owner }.abi_encode();
        assert_eq!(&call[..4], &[0x70, 0xa0, 0x82, 0x31]);
        // Selector then the left-padded 32-byte address word.
        assert_eq!(call.len(), 4 + 32);
        assert_eq!(&call[4..16], &[0u8; 12]);
        assert_eq!(&call[16..36], owner.as_slice());
    }

    #[test]
    fn u256_decodes_from_return_data() {
        // balanceOf returning 1234: a single 32-byte word.
        let mut data = vec![0u8; 32];
        data[30..32].copy_from_slice(&1234u16.to_be_bytes());
        let decoded = Erc20Reads::balanceOfCall::abi_decode_returns(&data).unwrap();
        assert_eq!(decoded, U256::from(1234u64));
    }
}
