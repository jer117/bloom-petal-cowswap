# Developing cowswap

Bloom serves `README.md` and `AGENTS.md` to every agent that uses this Petal,
so they hold only what a caller needs. Everything about building, testing and
releasing is here. Design decisions and verification vectors are in
`DESIGN.md`.

## Development skill

Petals are authored against the `bloom-petal-development` skill. `petal new`
wrote it to `.agents/skills/bloom-petal-development/SKILL.md`, byte for byte
from the contract crate of the CLI that generated the scaffold, so it is
available offline and matches that CLI. Load it into your agent before
extending this Petal. It is part of the Petal package, so editing or
refreshing it changes the package hash; Bloom does not serve it under
`/petals/cowswap/`.

## Building

Components build from the route crate's committed lockfile, with the petal CLI
of the SDK commit that lockfile pins. `scripts/build.sh` runs that CLI through
`scripts/petal.sh`, which installs it on first use, so the build never depends
on whichever `petal` is on PATH:

```sh
cargo test --manifest-path route/Cargo.toml --locked
scripts/build.sh
scripts/petal.sh check
```

Then validate the built package with `bloom petals build .`.

`route/Cargo.toml` pins the SDK by commit until the SDK's `v0.2.0` release is
tagged. To move to another SDK commit, change `rev` there, run
`cargo update --manifest-path route/Cargo.toml -p bloom-petal-sdk`, and set the
same commit in `.github/workflows/release.yml`. Once the tag exists, pin
`tag = "v0.2.0"` instead.

To build with a petal CLI you built yourself:

```sh
PETAL_BIN=/path/to/petal scripts/build.sh
```

Before pushing, enforce the route architecture rules from the
`bloom-petal-development` skill:

```sh
bash scripts/check-route-architecture.sh
```

## Testing on Sepolia

Sepolia is not one of Bloom's default chains. Add it to the Machine's
`config.toml` and restart Bloom:

```toml
[chains.sepolia]
name = "sepolia"
chain_id = 11155111
rpc_urls = ["https://ethereum-sepolia-rpc.publicnode.com"]
rpc_endpoints = []
display_name = "Sepolia"
native_symbol = "ETH"
native_decimals = 18
legacy_tx = false
op_stack = false
```

A packaged Bloom signs only for the Petals its installer catalogs, so run this
Petal on a developer triad. Bloom's `scripts/triad-dev-launch.sh` builds,
enrolls and installs it in one launch with
`BLOOM_TRIAD_DEV_HYPERLIQUID_PACKAGE=<this checkout>`; the development skill
has the details.

A full run needs a wallet with Sepolia ETH for gas and a token to sell. USDC
(`0x1c7D4B196Cb0C7B01d743Fbc6116a902379C7238`) for WETH
(`0xfFf9976782d46CC05630D1f6eBAb18b2324d6B14`) fills there. CoW's fee on
Sepolia is about 1 USDC, so sell more than that.

## Releasing

Pushing a `vX.Y.Z` tag runs `.github/workflows/release.yml`, which builds and
packages with the petal CLI of the pinned SDK commit and publishes the
archive, its checksum and `petal-release.json`. The workflow takes an SDK
commit only if it is on `bloom-directory/petal`'s `master` or is a release tag.
The commit pinned today is on a pull request branch, so a release has to wait
until that work is merged and the pin moves to the merged commit.
