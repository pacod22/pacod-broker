#!/usr/bin/env bash
# Copies the parts of a pinned commit of pacod-id the broker uses into vendor/pacod-id/, and
# packs its npm packages into ui/vendor/. Builds and Docker images then need nothing outside
# this repository: deny.toml refuses git sources. Adapted from pacod-gw's script of the same
# name; the broker takes only the three crates it builds on, not the whole service.
#
#   scripts/vendor-pacod-id.sh [<pacod-id checkout>] [<commit>]
#
# Defaults: the checkout next to this repository (../pacod-id) and its HEAD. Only committed
# files are taken (git archive). vendor/pacod-id/VERSION records the commit.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${1:-$(dirname "$ROOT")/pacod-id}"
REV="${2:-HEAD}"
CRATES=(pacod-id-authz pacod-id-client pacod-id-jose)
PACKAGES=(authz ui client sveltekit)

COMMIT="$(git -C "$SRC" rev-parse --verify "${REV}^{commit}")"
DEST="$ROOT/vendor/pacod-id"
rm -rf "$DEST"
mkdir -p "$DEST/crates"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
git -C "$SRC" archive "$COMMIT" | tar -x -C "$TMP"
for c in "${CRATES[@]}"; do
  cp -R "$TMP/crates/$c" "$DEST/crates/$c"
  # Their tests start the pacod-id server, which is not vendored here; pacod-id runs them.
  rm -rf "$DEST/crates/$c/tests" "$DEST/crates/$c/benches"
done
cp "$TMP/LICENSE" "$DEST/LICENSE"
{
  echo "commit $COMMIT"
  echo "date $(git -C "$SRC" show -s --format=%cI "$COMMIT")"
  echo "crates ${CRATES[*]}"
} >"$DEST/VERSION"

# The npm packages, built from the same commit and packed for the console.
if [ "${SKIP_NPM:-0}" != 1 ]; then
  (
    cd "$TMP"
    npm ci --no-audit --no-fund >/dev/null
    for p in "${PACKAGES[@]}"; do npm run build -w "packages/$p" >/dev/null; done
  )
  mkdir -p "$ROOT/ui/vendor"
  rm -f "$ROOT"/ui/vendor/pacod-*.tgz
  for p in "${PACKAGES[@]}"; do
    (cd "$TMP/packages/$p" && npm pack --pack-destination "$ROOT/ui/vendor" >/dev/null)
  done
fi
echo "vendored pacod-id $COMMIT"
