---
name: bloom-petal-development
description: Build, migrate, refactor, review, and validate Bloom Petals that expose domain behavior as route-based virtual files through the Petal SDK and builder. Use when turning an API or workflow into a Petal, creating a Petal, adding or changing route files, designing route and shared-code boundaries, declaring capabilities and manifest policy, investigating oversized route components, or preparing a Petal for packaging and release.
metadata:
  version: "0.2.0"
---

# Bloom Petal Development

Build Petals as explicit virtual files whose behavior is understandable from
their route source. Keep endpoint policy local and share only substantial,
typed infrastructure.

## Establish the local contract

1. Read the target repository's `AGENTS.md`, `README.md`,
   `petal-build.toml`, manifests, and build scripts before editing.
2. Inspect the exact route files and shared functions in scope.
3. Treat the target repository's pinned Petal SDK and tooling as authoritative.
   Do not assume another Petal uses the same SDK revision or helper API.
4. Preserve unrelated work in a dirty worktree.

When starting a new Petal, prefer the canonical `petal new` scaffold and adapt
its generated structure instead of inventing a parallel layout:

```sh
petal new <petal-name> <destination>
```

The command refuses an existing destination. Do not replace an existing
repository with a fresh scaffold.

## Resolve the product and trust contract

WIT defines the portable component interface. Bloom's installer provenance,
wallet policy and Broker's transaction review and approval ceremonies are
separate host semantics. Another host implementing the same WIT does not
acquire Bloom's trust or verification guarantees. State the tested host
combination when demonstrating signing.

Before coding, write down or recover these decisions from the user's request,
the target repository, and its domain/API documentation:

- the users and tasks the VFS should support;
- the proposed route tree and the content or body schema of every file;
- which reads contact an upstream, which writes have side effects, and which
  environments or networks are allowed;
- the lifecycle for every durable operation, including retry, approval,
  failure, recovery, and completion states;
- which credentials, wallet operations, settings, and stored values cross a
  trust boundary;
- the pinned Petal contract, SDK, CLI, and release target.

Do not invent a production write flow, credential format, signing intent, or
claim of completion. If one of those is genuinely unspecified, make the
read-only or sandbox choice, state the assumption, and ask only for decisions
that would materially change the route tree or trust boundary.

Use a compact route contract while designing:

| Route | Kind | Parameters | Read | Write body/effect | State | Capabilities |
| --- | --- | --- | --- | --- | --- | --- |
| `markets/[slug]/book.json` | file | `slug` | projected order book | — | none | `bloom:http` |

## Design the virtual filesystem first

Define each route's path, parameters, file kind, read/write behavior,
capabilities, cache behavior, and durable inspection surface before
implementing it. Prefer small files with one clear responsibility.

For each route file, keep these decisions local:

- route-parameter parsing and validation;
- upstream endpoint and request-body selection;
- response projection or user-facing read content;
- write-body parsing and route/action compatibility;
- the route's read description when a writable file is also readable;
- route-specific errors and semantics.

Use the route macro's local directory, `read`, or `read` plus `write` shape.
A writable route must define both handlers in that route file; use its read
handler for safe instructions or current non-secret state.

## Map route files explicitly

The configured `routes` directory is executable source, not a conventional Rust
module tree. Every `.rs` file below it becomes a separately compiled component:

- `route/files/$index.rs` defines the root directory;
- `route/files/markets/$index.rs` defines the `markets` directory;
- `route/files/markets/[slug]/book.json.rs` defines
  `markets/<slug>/book.json`;
- a bracketed segment such as `[slug]` is available through
  `petal::param(ctx, "slug")`;
- `[wallet]` and `[index]` are reserved for the route's own Bloom wallet and
  numbered account, and Bloom grants wallet authority only to the exact
  adjacent segments `<feature>/[wallet]/[index]/...`, for example
  `intents/[wallet]/[index]/new`, resolving both against its own wallet
  projection before the route runs. It refuses any other shape before the
  Petal runs: a `[wallet]` not immediately followed by `[index]`, such as
  `[wallet]/[account]`, an `[index]` not immediately preceded by `[wallet]`,
  and a suffixed capture, so `[wallet]/[index].json.rs` is refused too.
  `petal build` and `petal check` warn about such a shape but still succeed,
  and `bloom petals build` accepts it. An unknown wallet or account number is
  `not found`, never `permission denied`. Read the pair with
  `petal::wallet_account(ctx)`, which reads the trusted `bloom.wallet` and
  `bloom.account` parameters the host supplies. There is no implicit
  account 0, and every account, including 0, has its own private store;
- `<feature>/[wallet]/$index.rs` above those routes is
  `petal::account_listing!()`, because Bloom lists the accounts itself;
- the account's own on-chain address comes from Bloom's account projection,
  through `address_evm()` or `address_sol()` on the `petal::WalletAccount`, or
  its `address_evm_path()` or `address_sol_path()` leaf; never capture it as a
  route segment;
- `$list.rs` is unsupported; use `$index.rs`.

Directory contents are explicit. Use `list` for a fixed child list,
`fallible_list` for a computed result that does not need the context, and
`ctx_list` for a context-dependent result. Use `petal::dir`, `petal::file`, or
`petal::writable` to report the correct child kind. Listing a dynamic directory
does not happen automatically.

Validate parameters for their domain before using them in URLs, storage keys,
or VFS paths. Before interpolating a value into a URL, enforce its strict
domain grammar or encode it as a URL path segment. Do not assume supplied
context parameters are safe merely because the source filename uses brackets.

- The route's own wallet and account come from `petal::wallet_account(ctx)`,
  which reads the pair Bloom resolved before dispatch. Build paths under it
  with its bounded `path()`.
- A wallet id from a request body or stored state goes through
  `petal::validate_wallet_segment` before it becomes a VFS path segment or a
  store key. `petal::validate_wallet_id` is wider, admitting `.` and `/`, and
  is only for the wire boundary.
- `petal::is_safe_segment` judges one segment and admits `/`, so it is not a
  wallet-id check. Split any multi-segment value on `/` and check each part.
  It is not URL validation either.

## Select the local handler and spec

Use one supported `petal::route_file!` shape per route:

- directory: `list`, `fallible_list`, or `ctx_list`;
- read-only file: `read`;
- writable file: `read` plus `write`.

Start with the narrowest matching spec:

| Spec | Intended use | Default declared capabilities |
| --- | --- | --- |
| `static_dir_spec()` | fixed directory | none |
| `store_dir_spec()` | stored/dynamic directory | store, VFS read |
| `http_dir_spec()` | upstream-backed directory | HTTP |
| `static_read_spec()` | pure/static file | none |
| `http_read_spec(ttl_ms)` | cacheable upstream read | HTTP |
| `store_read_spec()` | stored read | store |
| `wallet_http_read_spec(ttl_ms)` | HTTP read using wallet VFS data | HTTP, VFS read |
| `account_read_spec()` | HTTP/store/wallet account read | HTTP, store, VFS read |
| `chain_read_spec()` | side-effecting, uncached chain flow | broad host set |
| `write_spec()` | uncached writable file | broad host set |
| `signing_write_spec(intent)` | writable file with signing intent | broad host set |

Override with `.caps(&[...])` when the compiled route imports a narrower or
different set. The broad write and chain specs are convenience defaults, not
permission to retain unused capabilities.

Return errors through `petal::error`: `-1` not found, `-2` denied, `-3`
invalid input, and `-4` backend failure. Other codes become unsupported.
Sanitize upstream and storage errors before returning them.

## Avoid path-based dispatch

Do not create catch-all `read`, `write`, or `dispatch` functions that inspect
route identity. This covers every accessor the SDK exposes now or later,
including `current_route_path` and `current_route_canonical_path`, as well as
filenames, path suffixes, and parameter presence. Do not route through chains
of `contains`, `ends_with`, or string-keyed matches.

The structural guarantee is that each route file is generated as its own
component crate and the `route_file!` macro wires exactly one local handler
per file, so shared code cannot serve a route unless that route opts in. Treat
this as the primary enforcement. The toolchain's `petal check` rejects route
identity accessors in the shared route crate and reconciles every route's
declared capabilities against its compiled imports. Repository-level checks
may enforce additional local rules, but must not duplicate the SDK accessor
denylist.

Avoid:

```rust
pub fn read(ctx: &petal::Ctx) -> petal::DispatchResponse {
    let path = petal::current_route_path(ctx);
    if path.ends_with("status.json") {
        // ...
    } else if path.contains("orders/") {
        // ...
    } else {
        // ...
    }
}
```

Prefer:

```rust
petal::route_file!(
    spec: petal::http_read_spec(5_000),
    read: |ctx: &petal::Ctx| {
        let market = match petal::param(ctx, "market") {
            Ok(market) => market,
            Err(response) => return response,
        };
        read_market_status(market)
    },
);
```

This keeps route behavior discoverable and allows each independently compiled
component to discard code it does not use.

## Share typed infrastructure

Put code in the shared route crate only when it represents substantial,
reusable behavior rather than endpoint selection. Good shared candidates
include:

- protocol types, serialization, hashing, and signing;
- bounded HTTP, store, chain, transaction, and VFS adapters;
- idempotency and durable state machinery;
- domain validation or policy used identically by multiple routes;
- multi-step operations with typed inputs and outputs.

Pass typed values from the route file into shared functions. Avoid passing a
route path or filename and asking shared code to infer the operation. A small
amount of repetition in route files is preferable to hidden coupling through
dynamic dispatch.

## Declare least capabilities

Keep three scopes distinct:

- the route spec declares the component's `required_caps`;
- `[caps].allowed` in `petal.toml` declares the package-wide ceiling;
- `[[net.allow]]`, `[sign]`, and `[store]` constrain particular host surfaces.

Make each route's metadata capabilities the authority it intentionally needs
at runtime, within the ceiling inferred from its compiled imports. Make the
package ceiling the union of capabilities the Petal intentionally uses.

Bloom derives `bloom:vfs.read` and `bloom:vfs.write` from the individual
`bloom:vfs/readwrite` functions a component imports, while `petal check` counts
any `bloom:vfs/readwrite` import as both. So a route with parameters that
declares `bloom:vfs.write` but imports only read functions passes
`petal build`, `petal check`, `bloom petals build` and install, then fails
every call with
`invalid: … metadata requires missing petal.toml cap bloom:vfs.write`, even
when `[caps].allowed` lists it: the message names `petal.toml`, but the check
is against the route's imports. Remove the capability from the route's
metadata. If such a route also imports `bloom:sign`, `bloom petals build` and
install check its metadata against `[caps].allowed` as well, but it still
fails the same way at runtime. A route without parameters is checked against
`[caps].allowed` at build and install only, and Bloom silently drops any
metadata capability its imports lack.

Refactoring shared dispatch into local handlers often removes imports through
dead-code elimination, so revisit both scopes after architectural changes.

Use the narrowest applicable route spec, then override capabilities only when
the route needs a different set. It is valid for route metadata to narrow the
compiled import ceiling; Bloom grants only the narrowed set, so any omitted
host surface must be unreachable during successful execution.

Know the validator boundaries:

- Petal CLI `build`, `check`, and `package` reject a capability declared by a
  route spec when the compiled component does not import it, and reject unknown
  component imports.
- Petal CLI packaging validates only basic `petal.toml` identity; it does not
  prove that the complete manifest policy covers compiled imports.
- Bloom `petals build` and install validation reject compiled imports not
  covered by `[caps].allowed` and the applicable network, signing, and store
  policy. They also enforce route metadata as a runtime narrowing boundary.

Inspect the top-level `bloom:*` imports of changed route components with
`wasm-tools component wit`, then compare them with the route spec and package
manifest. Treat unexpected imports or authority as architecture feedback, not
as a reason to broaden declarations.

Import-to-capability mapping is:

- `bloom:http/fetch` → `bloom:http`;
- `bloom:store/kv` → `bloom:store`;
- `bloom:sign/signing` → `bloom:sign`;
- `bloom:tx/outbox` → `bloom:tx.outbox`;
- `bloom:chain/read` → `bloom:chain`;
- `bloom:vfs/readwrite` → VFS read and/or write according to used functions;
- `bloom:env/runtime` → no declared capability.

The Petal CLI currently treats a VFS interface import as both VFS capabilities
when checking a route's declared ceiling; Bloom's package validator performs
the finer used-function inference. Confirm the final policy with Bloom rather
than inferring it from the interface name alone.

Network egress is part of least capability. Declare every reachable upstream
host in `petal.toml` with a separate `[[net.allow]]` table, explicit methods,
and path prefixes; never rely on a wildcard host. A wildcard host such as
`*.example.com` installs but matches no request, and a path without a trailing
`*` matches only itself: `/v1` does not allow `/v1/x`, while `/v1/*` does.
Shared HTTP helpers should
resolve targets from a fixed, enumerated source (for example a `Network`
variant that returns a `&'static str` URL) so route code cannot direct a
request at an arbitrary host. If a new upstream is introduced, widen
`[net.allow]` deliberately, not as a side effect of editing a helper.

### Delegated child keys

A route that imports `bloom:key/derive@0.1.0` and hands the derived key to
other routes declares that scope on its exact route pattern:

```toml
[[key.derive]]
route = "[network]/agent_sessions/[wallet]/[index]/new.json"
operation_classes = ["venue.agent_action"]
allowed_routes = ["[network]/orders/[wallet]/[index]/new.json"]
allowed_crypto_suites = ["secp256k1-keccak256-recoverable"]
maximum_lifetime_ms = 86400000
```

Every listed class must also appear in `[sign].allowed_intents`. Declare all
three bounds. A component may later request fewer operation classes or crypto
suites, or a shorter lifetime, never more; `allowed_routes` always applies as
declared, plus the declaring route itself. `allowed_crypto_suites` takes
exactly the Broker `CryptoSuite` values: `secp256k1-keccak256-recoverable`
(what EVM chains verify), `secp256k1-sha256-recoverable` (for verifiers that
hash with SHA-256; no EVM or Solana transaction or message format uses it) and
`ed25519-message` (the only suite a Solana wallet permits).

## Preserve write semantics

Through `bloom vfs write` (and MCP), Bloom runs a route's `write` synchronously
and returns its error to the caller; a successful write means the handler
finished, nothing more. Through the mount, a component route that exports
`write` runs as a detached command: the SDK's write specs set `write_async`,
and Bloom also sets it at install. The writer sees success at once, and a
failure reaches only Machine's log. Record the outcome in a readable file
rather than relying on the write's return.

A guest VFS call can never reach another Petal or re-enter its own: every
`petals/…` path is denied, and wallet paths are limited to the selected
numbered account.

For side-effecting routes:

- validate body size and shape before external work;
- bind the body to the route's permitted operation;
- preserve stable idempotency identifiers across approval retries;
- store enough status, response, and error state for inspection;
- distinguish approval, staging, submission, broadcast, confirmation, and
  domain completion;
- never report a broadcast, confirmation, or fill solely because a route write
  was accepted.

Do not exercise production writes or material funds unless the user explicitly
authorizes that environment and action.

## Protect sensitive material

Private keys, derived agent keys, API secrets, bearer tokens, secret-store
values, and replayable or not-yet-submitted signatures are confidential.
Store them only in the store's secret namespace and never copy their bytes
into the public state namespace, errors, logs, or a read response. Treat
signature bytes as confidential unless the product contract explicitly
classifies a finalized signature as public. When a read needs to expose signing
state, return only what an auditor needs: status, intent, outcome, and
non-secret identifiers.

When a route signs locally with a stored key, the route's declared capabilities
should omit the host signing capability and the signing function must live
behind a typed helper that cannot return key bytes to its caller. Treat the
boundary between the secret namespace and the readable VFS as a hard wall and
add tests that confirm public routes and packaged artifacts contain no secret
value. Use the pinned SDK or host binding for secret reads; do not assume
`petal::sdk::store_get` reads the secret namespace.

## Validate in layers

Use the repository's pinned commands and scripts. At minimum:

1. Format and run host-side unit tests.
2. Run clippy or the repository's equivalent static checks.
3. Run the repository's route-architecture source check.
4. Run `petal build --root <root>` to compile every route component. Run it
   from the Petal root: Cargo reads `.cargo/config.toml` from the directory it
   runs in, not from `--root`. It builds from the route workspace's
   `Cargo.lock`, which the step 1 tests write; commit that lockfile.
5. Run `petal check --root <root>` against the just-built components.
6. Inspect changed components' imports and compare all three policy scopes.
7. Run `petal package --root <root> --out <versioned>.petal.tar.gz` or the
   repository's release-validation command. Packaging refuses to overwrite an
   existing archive.
8. Run the pinned Bloom package-validation command, normally
   `bloom petals build <root>` against a running Machine. Only it catches a
   compiled import whose capability is missing from `[caps].allowed`, a
   `sign_intent` missing from `[sign].allowed_intents`, or a reserved segment
   such as `$index` used as a directory. Then exercise the target repository's
   VFS smoke tests when available.
9. Confirm the expected route count and inspect unexpected component-size
   changes.

Do not use a successful host-crate build as evidence that route sources compile;
route files are independently generated component crates. `petal check` does
not rebuild, so do not use it against stale artifacts. `petal package` rebuilds
current sources before packaging, but does not fully validate `petal.toml`
policy and does not replace route-level tests or a runtime VFS smoke test. A
rebuild sees only the current environment: when the Petal's build command
supplies compile-time values, such as a key read with `option_env!`, run that
command and then `petal package --no-build`, which packages the components it
produced. The reusable release workflow does this. `--no-build` refuses a
component with no route controller, but like `petal check` it cannot tell that
sources changed after the build, so run it straight after the build command.

## Run it in Bloom

Validate and install with the Bloom build you will run against. `build`
regenerates `artifacts/routes/` and `artifacts/build-manifest.json`; never
hand-edit them or install stale ones.

```sh
bloom petals build <root>
bloom petals build <root> --out <name>.petal.tar
bloom petals install <root>
bloom petals install <name>.petal.tar
bloom petals install https://github.com/bloom-directory/<repo> --ref <tag-or-sha>
bloom petals ls
bloom petals uninstall <name-or-hash-prefix>
```

Know what an install changes:

- When Bloom hashes a package directory (`bloom petals build`, or
  `bloom petals install <root>`), it covers every file under the root except
  the top-level `.git` and `.jj` directories and every directory named
  `target`, at any depth. Those are left out of the package itself, so never
  name a route directory `target`; a `.git` file, as in a Git worktree, is
  included. Any changed or added file, such as a stray note, archive or
  `.zip`, changes that hash (`bloom petals build` regenerates
  `artifacts/routes/` and `artifacts/build-manifest.json` before it hashes
  them). The archive `petal package` writes holds only the runtime inputs:
  `petal.toml`, `README.md`, `AGENTS.md`, `petal/<name>/` (Bloom refuses any
  other entry under `petal/`), the optional `components/`, `modules/` and
  `wit/` package directories (route sidecars compose from the first two), and
  the development skill at `.agents/skills/bloom-petal-development/SKILL.md`,
  each under exactly that name, with each directory a real directory.
  Other files change neither the archive nor the hash Bloom computes when it
  installs it. Pass a deliberate runtime tree or a validated runtime archive,
  rather than the whole development checkout, when that is what you intend to
  ship. Reproducibility depends on the pinned builder, dependency graph,
  compiler and flags: compare actual component, composed-package and archive
  bytes across directories instead of inferring identity from source commits
  or checkout locations. The `petal build` released with this contract builds
  from the route workspace's committed `Cargo.lock`, refusing any change to the
  dependency graph, and keeps the paths of the checkout, the Cargo home and
  the Rust toolchain out of its components. It reaches the route crate through
  a link inside its generated workspace, because Cargo hashes a path package's
  absolute location into symbol names otherwise. The link points at the
  deepest directory holding the Petal and its path dependencies, so a path
  dependency outside the Petal makes the directory names below that point
  part of the components, and a Cargo config `[patch]` to a local SDK ties
  them to that SDK's location. Without those, the same sources, compiler and
  flags give the same components in another directory; another compiler,
  other flags or another builder give new ones.
  The builder sets its release profile on the Cargo command line and
  `CARGO_INCREMENTAL=0`, so `[profile.release]` and `build.incremental` in
  Cargo config and `CARGO_PROFILE_RELEASE_*` variables do not change
  components, but a per-package profile override does. Each build moves the
  output it replaces under `target/petal-routes/staging/` and removes it only
  when it holds exactly the components an earlier build recorded writing to
  that output, apart from Finder's `.DS_Store`; it keeps anything else there,
  such as a file it did not write, for you to inspect and delete. A build that
  stops with an error removes what it staged; a killed one can leave its
  directory there. With `RUSTFLAGS` or `CARGO_ENCODED_RUSTFLAGS`
  set, the builder adds its remapping to `CARGO_ENCODED_RUSTFLAGS`. Otherwise
  it adds it to `CARGO_BUILD_RUSTFLAGS`, so target-specific rustflags, which
  Cargo uses instead of `build.rustflags`, drop it. Cargo splits that
  variable on whitespace, so for a path containing whitespace the builder uses
  a `--config` array, which Cargo cannot merge into a string
  `build.rustflags` from Cargo config. The build then stops before compiling
  and says so; write that as an array.
  Propose installer provenance and wallet policy only for the hash of the exact
  package that will be installed: the hash `bloom petals build` reports for
  the package directory you will install as it is (or for the archive it
  writes with `--out`), or, for a release built by the reusable release
  workflow, the `package_hash` in its `petal-release.json`. That is the
  package hash of the published archive's contents, not its `archive_sha256`,
  and Bloom checks it when it installs a release pinned in its built-in
  catalog. Building does not enroll a hash. Wallet policy allows Petal
  packages by hash (`allowed_petal_packages`), so the owner approves a policy
  update for a new hash before its routes can sign, derive keys or stage
  transactions for that wallet; reads do not need it.
- A new hash starts with an empty private store, unless it is a signed
  successor in an authenticated lineage. Bloom then copies the predecessor's
  store bytes on first use, with no migration hook, so a successor must read
  the previous release's format. Test against a fixture store it wrote,
  including settings and secrets.
- A signing route is authorized only for the operation classes its installer
  provenance record lists. Without that record the write fails with a plain
  permission error; for `sign_payload` with the exact selector, Machine's log
  names the reason (`installer provenance does not authorize <class>`). See
  [Troubleshoot a bare `permission denied`](#troubleshoot-a-bare-permission-denied).
- `--force` is not a reinstall flag. It replaces or removes a package while
  sessions scoped to it are still active; those sessions then read
  `package_replaced`, and only Exact recovery remains.
- A GitHub source package may declare `[source]` and `[build].command`. Bloom
  runs that command natively with the installing user's privileges and does
  not sandbox it, so keep the build non-interactive and dependency-light, and
  test installs by tag and by immutable commit. Bloom builds each install in a
  fresh temporary checkout with the installing machine's toolchain and Cargo
  config, so its hash depends on that machine and cannot be approved in
  advance; publish a release archive, and install that archive, when the hash
  must be known before install.

Exercise the installed tree both through direct VFS
(`bloom vfs ls|cat|write /petals/<name>/...`) and through a mounted daemon:
lookup, list, read, invalid input, backend failure, and every writable route.

### On a developer triad

Bloom's `scripts/triad-dev-launch.sh` installs two optional packages after the
services start: `BLOOM_TRIAD_DEV_POLYMARKET_PACKAGE` and
`BLOOM_TRIAD_DEV_HYPERLIQUID_PACKAGE`. `BLOOM_TRIAD_DEV_BUILD_PETALS=0` skips
building the Hyperliquid package; the Polymarket package is installed as it is.
Only the Hyperliquid hook also enrolls installer provenance. For any other Petal
that signs, enroll it before launching:

```sh
bloom init triad-enroll-developer-petal-provenance <developer-root>/config <root>
```

The command exists only in the `triad-dev-harness` build of `bloom` that
`scripts/triad-dev-launch.sh` builds (`<bloom-checkout>/target/debug/bloom`); a
packaged `bloom` does not have it.

Enrollment hashes the package directory the way `bloom petals install` does,
from the components already built under `petal/<name>/`, so run `petal build`
first and install the same files afterwards. Any file changed or added under
the root since enrollment changes the hash, and the provenance no longer
matches. Machine and Broker read the catalog at startup, so enroll before the
triad starts or restart it afterwards.

## Troubleshoot a bare `permission denied`

Bloom drops the message of every denial. A route response with code `-2`
(`RouteError::Denied`) reaches the caller only as `permission denied`: EACCES
on the mount, JSON-RPC `-32007` through `bloom vfs`. The SDK reports a host
denial as `SdkError::Host(HostStatus::Denied)`, and the route chooses the code:
a route that maps it to `-2` gives a bare `permission denied`, while SDK
helpers such as `petal::read_store` return `-4`, which reaches the caller as
`backend: denied` (EIO on the mount). Work through the candidates:

- **Route shape.** A `[wallet]` not immediately followed by `[index]`, an
  `[index]` not immediately preceded by `[wallet]`, or a suffixed capture such
  as `[index].json` draws a `warning: route …` from `petal build` and
  `petal check` but installs without error. Bloom then refuses every call to it
  at dispatch and logs nothing. The route still appears in directory listings,
  so a listed file can be unreadable. The exception is a directory route ending
  at `[wallet]` with `[wallet]/[index]/...` routes beneath it: Bloom lists the
  accounts itself, so its `$index.rs` never runs, and reading it is denied.
- **Owner approval pending.** Bloom does not deny a signature that needs
  approval. The SDK returns `ApprovalPending` with an action id. For a
  signature, the owner-only `/petal-signing-requests/<id>.json` carries the
  `ceremony_url`; when the wallet policy must first allow the package hash, the
  id names that policy update instead. It becomes `permission denied` only when
  the route returns `-2` for it, and then its message is lost, so record the
  pending state in a readable file. After the owner approves, retry the exact
  same write. A retry that changes the payload or the claim, including the
  claim's nonce or effects, is a new request with its own approval. A retry
  that keeps the claim but changes its `action` or `advisory` is refused
  (`exact signing retry differs from its persisted operation identity`). A
  retry before the owner approves is refused (`approval is not active`), and
  one five minutes after the first attempt opens a new ceremony.
  Do not pass the pending action id back as `approval_hint`; a hint other than
  the derived request id is refused.
- **One live ceremony per wallet.** While an owner ceremony is pending, a
  signing request on that wallet that needs a new ceremony is refused with
  `CEREMONY_RATE_LIMITED`
  (`wallet already has a live ceremony; retry after <n> ms`). The same code
  also reports a cancellation backoff and the rolling per-wallet quota, for
  which Broker also logs `ceremony.quota_rejected` at WARN. The route sees only
  a denial. Machine's `petal.sign_payload_denied` line carries the code, in
  `reason` with the full message on the exact path and in
  `protocol_error_code` on the reusable path; Broker's `rpc.completed` carries
  the code only. When the refused ceremony is the policy update for a new
  package hash, Machine logs nothing, and only `rpc.completed` shows the code.
  Finish or wait out the pending ceremony before the next signing write.
- **Signing refused.** Missing installer provenance for the operation class, a
  wallet policy that does not allow the package hash or a destination, an
  approval that is not yet active, spent or expired, or a reused approval hint.
  For `sign_payload`, Machine logs most of these at WARN as
  `petal.sign_payload_denied`. On the exact path the line carries `wallet` and
  `operation_class`, usually with `package_hash`, `route`, `request_id` and a
  `reason`; when Broker refuses a reusable request, it carries only
  `package_hash`, `route` and `protocol_error_code`. On the exact path Machine
  also logs Broker's refusal at ERROR as `petal exact signing failed`, with its
  code and message. `sign_payload_batch` refusals are not logged under that
  name. A batch refused by Machine, for example for missing installer
  provenance, leaves no log line at all; Broker errors on the exact path appear
  at ERROR as
  `petal exact batch signing failed`, and the reusable path logs nothing.
  Refusals checked inside the route runtime, such as an operation class outside
  the route's `sign_intent` or `[sign].allowed_intents`, a signing route without
  `[wallet]/[index]`, or a wallet that differs from the routed one, are not
  logged either.
- **Capabilities and policy.** A host import whose capability the route does
  not declare is logged at INFO as `host capability denied`, with the `op`. An
  HTTP request outside `[[net.allow]]`, including one under a wildcard host or
  below a path rule without a trailing `*`, is logged at INFO as
  `component http.fetch denied by net policy`. A store namespace or visibility
  mismatch, `put_new` on an existing key, a VFS path outside the selected
  account or under `petals/`, and a chain method outside the read allowlist are
  denied without a log line.
- **Mounted writes.** Through the mount, a write to a route runs
  asynchronously: the writer sees success, and a failure reaches Machine's log
  only at WARN as `mount.adapter.async_command_outcome_deferred`. Reproduce with
  `bloom vfs write` to see the error. A write to a route without a write
  handler fails the mount's access check with EACCES before the route runs;
  through `bloom vfs` it is `invalid: mode unsupported`. A refused route shape
  fails at once on the mount too, because its lookup is denied.

Machine's log is the place to look:

| Machine runs from | Log |
| --- | --- |
| `scripts/triad-dev-launch.sh` | `<log-dir>/machine.log`, text |
| Packaged macOS | `~/.bloom/logs/machine.jsonl`, JSON Lines |
| Packaged Linux | `journalctl --user -u bloom-machine.service -o cat`, JSON Lines |

In JSON Lines the event name is the tracing message, under `.fields.message`:

```sh
grep -a 'petal.sign_payload_denied' <log-dir>/machine.log
jq -cR 'fromjson? | select(.fields.message == "petal.sign_payload_denied")' \
  ~/.bloom/logs/machine.jsonl
```

Broker refusals also appear in the Broker log as `broker.authority_rejected`,
`broker.signer_operation_rejected`, or `rpc.completed` with a
`protocol_error_code` such as `CEREMONY_RATE_LIMITED`: `<log-dir>/broker.log` on
a developer triad, `/private/var/log/bloom/<uid>/broker.jsonl` on macOS, and
`journalctl -t bloom-broker-<uid> -o cat` on Linux. Broker's JSON Lines carry
the event name under `.fields.event`, not `.fields.message`:

```sh
jq -cR 'fromjson? | select(.fields.event == "broker.authority_rejected")' \
  /private/var/log/bloom/<uid>/broker.jsonl
```

Route-shape refusals and the unlogged denials above leave nothing in either
log, so check the route pattern, its declared capabilities and its manifest
grants first.

## Review checklist

Before handing off, confirm:

- every route's behavior is visible in its route file;
- shared code contains no filename- or path-driven endpoint dispatcher;
- every writable file defines safe local read behavior;
- route capabilities stay within compiled imports and cover successful paths;
- the package capability ceiling and per-surface policy cover only intended use;
- network egress is allowlisted to the narrowest host, method, and path set;
- no secret or signature material is reachable through a read handler;
- durable state does not overstate external completion;
- all route components and release packaging validate;
- the package was installed in Bloom and exercised through the VFS;
- no live write was performed without explicit authorization.
