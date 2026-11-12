#!/bin/sh
# Run the petal CLI of the SDK commit this Petal pins.
#
# route/Cargo.toml names the SDK tag or commit, and route/Cargo.lock records
# the commit. The first time it sees that commit, this installs the petal CLI
# from it, with the wasm-tools release it builds with, under
# ~/.cache/bloom-petal. For a tag it first checks that the tag names the
# commit. So the SDK pin is the only version to change. PETAL_BIN names
# another binary, as the release workflow does.
#
# This differs from the SDK scaffold's script in one way: it also takes a
# commit pin (`rev = "..."`), for use until the SDK release is tagged.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

if [ -n "${PETAL_BIN:-}" ]; then
  exec "$PETAL_BIN" "$@"
fi

lock=route/Cargo.lock
if [ ! -f "$lock" ]; then
  echo "petal.sh: $lock is missing; create it with: cargo test --manifest-path route/Cargo.toml" >&2
  exit 1
fi
source=$(awk '
  /^\[\[package\]\]/ { sdk = 0 }
  $0 == "name = \"bloom-petal-sdk\"" { sdk = 1 }
  sdk && /^source = / { sub(/^source = "/, ""); sub(/"$/, ""); print; exit }
' "$lock")
repo=https://github.com/bloom-directory/petal
tag=
case "$source" in
  "git+$repo?tag="*"#"* | "git+$repo.git?tag="*"#"*)
    pin=${source#*\?tag=}
    tag=${pin%%#*}
    commit=${pin##*#}
    ;;
  "git+$repo?rev="*"#"* | "git+$repo.git?rev="*"#"*)
    commit=${source##*#}
    ;;
  "")
    # A Cargo config [patch] puts the SDK at a local checkout: use the petal
    # CLI on PATH, built from that checkout.
    exec petal "$@"
    ;;
  *)
    echo "petal.sh: $lock takes bloom-petal-sdk from $source; pin a release tag or a commit of $repo, or set PETAL_BIN to a petal CLI you built" >&2
    exit 1
    ;;
esac
# The tag and commit go into a Git ref and a path below, so take only a
# release tag and a full commit.
if [ -n "$tag" ]; then
  case "$tag" in
    v[0-9]*) ;;
    *) commit= ;;
  esac
  case "$tag" in
    *[!0-9A-Za-z.+-]*) commit= ;;
  esac
fi
case "$commit" in
  *[!0-9a-f]*) commit= ;;
esac
if [ "${#commit}" -ne 40 ]; then
  echo "petal.sh: $lock does not pin bloom-petal-sdk to a release tag or a full commit: $source" >&2
  exit 1
fi

tools=${PETAL_TOOLS_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/bloom-petal}/$commit
install_cli() {
  if [ ! -x "$tools/bin/petal" ]; then
    echo "petal.sh: installing the petal CLI at $commit into $tools" >&2
    cargo install --locked --quiet --git "$repo" --rev "$commit" --root "$tools" bloom-petal-cli
  fi
}
if [ -n "$tag" ]; then
  # GitHub serves a commit from any fork of the repository at its URL, so run
  # only the commit the tag names. A file per tag records that it named this
  # commit when checked. Without one, check, even with a CLI already here: an
  # earlier petal.sh installed without checking, and the CLI may have been
  # installed for another tag.
  checked=$tools/checked-tags/$tag
  if [ ! -f "$checked" ]; then
    if ! refs=$(git ls-remote "$repo" "refs/tags/$tag" "refs/tags/$tag^{}"); then
      echo "petal.sh: could not read $tag from $repo" >&2
      exit 1
    fi
    named=$(printf '%s\n' "$refs" | awk -v ref="refs/tags/$tag" '
      $2 == ref { plain = $1 }
      $2 == ref "^{}" { peeled = $1 }
      END { print (peeled != "" ? peeled : plain) }
    ')
    if [ "$named" != "$commit" ]; then
      echo "petal.sh: $lock pins $tag at $commit, but $repo names ${named:-no commit} for $tag" >&2
      exit 1
    fi
    install_cli
    mkdir -p "$tools/checked-tags"
    : >"$checked"
  fi
else
  # A commit pin names the commit outright, so there is no tag to check it
  # against. This is the commit Cargo already builds the SDK from. The release
  # workflow takes it only if it is on the SDK repository's master.
  install_cli
fi

# Build with the wasm-tools release this CLI is tested with.
wasm_tools=$("$tools/bin/petal" inspect | sed -n 's/^wasm_tools: //p')
if [ -n "$wasm_tools" ] && [ "$(wasm-tools --version 2>/dev/null || true)" != "wasm-tools $wasm_tools" ]; then
  if [ "$("$tools/bin/wasm-tools" --version 2>/dev/null || true)" != "wasm-tools $wasm_tools" ]; then
    echo "petal.sh: installing wasm-tools $wasm_tools into $tools" >&2
    cargo install --locked --quiet --version "=$wasm_tools" --root "$tools" wasm-tools
  fi
  PATH="$tools/bin:$PATH"
  export PATH
fi

exec "$tools/bin/petal" "$@"
