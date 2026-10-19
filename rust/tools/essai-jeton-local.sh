#!/usr/bin/env bash
# SPDX-License-Identifier: EUPL-1.2
# Essai de bout en bout du jeton court, sans AWS : le PHP de app.runeride.fr (classe Jwt) signe des jetons au format de
# POST /api/balade/jeton, la Lambda Raido (binaire bootstrap, mode --evenement) les vérifie contre le JWKS et applique
# les paliers (spec runeride/docs/itineraire.md §6.4, §7.1). 13 cas, résultats attendus dans essai-jeton/cas.py.
#
#   rust/tools/essai-jeton-local.sh <runeride/web> <dossier du graphe> [dossier de travail]
#
# Prérequis : php (openssl), python3, cargo (compile bootstrap en release si besoin). Rien n'est envoyé nulle part.
set -euo pipefail
WEB=${1:?chemin de runeride/web}; GRAPHE=${2:?dossier du graphe (index.json)}; DIR=${3:-$(mktemp -d)}
RACINE=$(cd "$(dirname "$0")/.." && pwd)        # …/rust
OUTILS="$RACINE/tools/essai-jeton"
[[ -f "$WEB/src/Auth/Jwt.php" ]] || { echo "Jwt.php introuvable sous $WEB" >&2; exit 2; }
[[ -f "$GRAPHE/index.json" ]] || { echo "index.json introuvable sous $GRAPHE" >&2; exit 2; }
B="$RACINE/target/release/bootstrap"
[[ -x "$B" && "$B" -nt "$RACINE/crates/raido-lambda/src/jeton.rs" && "$B" -nt "$RACINE/crates/raido-lambda/src/handler.rs" ]] \
  || (cd "$RACINE" && cargo build --release --locked -p raido-lambda --bin bootstrap)
php "$OUTILS/jetons.php" "$WEB/src/Auth/Jwt.php" "$DIR"
python3 -I "$OUTILS/cas.py" "$DIR"
export RAIDO_SOURCE="local:$GRAPHE" RAIDO_CACHE="" RAIDO_JETON=obligatoire RAIDO_JWKS_FICHIER="$DIR/jwks.json" \
       RAIDO_EMETTEUR=https://app.test RAIDO_AUDIENCE=raido RAIDO_THREADS="${RAIDO_THREADS:-4}"
for f in "$DIR"/evt_*.json; do
  n=$(basename "$f" .json); n=${n#evt_}
  "$B" --evenement "$f" > "$DIR/rep_$n.json" 2> "$DIR/journal_$n.log"
done
python3 -I "$OUTILS/resume.py" "$DIR"
echo "dossier de travail : $DIR (clé et jetons jetables)"
