#!/usr/bin/env bash
# SPDX-License-Identifier: EUPL-1.2
# Construit le binaire de la fonction Lambda Raido et l'emballe dans un zip (rien n'est envoyé nulle part).
#
#   rust/tools/package-lambda.sh            # arm64 (Graviton), cible aarch64-unknown-linux-musl
#   rust/tools/package-lambda.sh x86_64     # x86_64 musl, pour comparer
#
# Sortie : rust/target/lambda/raido-lambda-<arch>.zip contenant `bootstrap` (0755, à la racine) + somme SHA-256.
# Même procédé que runeride-weather/rust/tools/package-lambda.sh : aucun compilateur C croisé (TLS en Rust pur,
# éditeur de liens rust-lld fourni avec la chaîne Rust). Prérequis : rustup target add aarch64-unknown-linux-musl.
set -euo pipefail
ARCH=${1:-arm64}
case "$ARCH" in
  arm64)  CIBLE=aarch64-unknown-linux-musl ;;
  x86_64) CIBLE=x86_64-unknown-linux-musl ;;
  *) echo "architecture inconnue : $ARCH (arm64 | x86_64)" >&2; exit 2 ;;
esac
RACINE=$(cd "$(dirname "$0")/.." && pwd)        # …/rust
cd "$RACINE"
RUSTFLAGS="${RUSTFLAGS:-} -C linker=rust-lld" cargo build --release --locked -p raido-lambda --bin bootstrap --target "$CIBLE"
BIN="target/$CIBLE/release/bootstrap"
OUT="target/lambda"
mkdir -p "$OUT"
TMP=$(mktemp -d)
install -m 0755 "$BIN" "$TMP/bootstrap"
ZIP="$PWD/$OUT/raido-lambda-$ARCH.zip"
rm -f "$ZIP"
# -X : sans horodatages ni attributs supplémentaires (zip reproductible à contenu égal) ; le mode 0755 est conservé
(cd "$TMP" && touch -d @0 bootstrap && zip -q -X -9 "$ZIP" bootstrap)
rm -rf "$TMP"
echo "binaire : $BIN ($(stat -c %s "$BIN") octets)"
echo "zip     : $ZIP ($(stat -c %s "$ZIP") octets)"
(cd "$OUT" && sha256sum "raido-lambda-$ARCH.zip")
