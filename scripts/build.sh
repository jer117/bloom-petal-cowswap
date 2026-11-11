#!/bin/sh
# Build every route component into petal/cowswap/. The release workflow
# runs this with PETAL_BIN set to the CLI it built; elsewhere scripts/petal.sh
# runs the CLI of this Petal's SDK release.
set -eu
exec "$(dirname -- "$0")/petal.sh" build
