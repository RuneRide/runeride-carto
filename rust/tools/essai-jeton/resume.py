# SPDX-License-Identifier: EUPL-1.2
# Tableau des réponses rejouées (rep_*.json, journal_*.log) et comparaison avec attendu.json. Sortie 1 si un cas diverge.
#   python3 -I resume.py <dossier>
import glob, json, os, re, sys
d = sys.argv[1]
attendu = json.load(open(os.path.join(d, "attendu.json")))
ko = 0
print(f"| Cas | Statut | Code | Détail |\n|---|---|---|---|")
for f in sorted(glob.glob(os.path.join(d, "rep_*.json"))):
    n = os.path.basename(f)[4:-5]
    r = json.load(open(f))
    b = json.loads(r.get("body") or "{}")
    statut = r.get("statusCode")
    detail = b.get("erreur") or (f"{b.get('distance_m')} m, {b.get('duree_s')} s" if "distance_m" in b else "état " + str(b.get("etat")))
    m = re.search(r'"ms":(\d+)', open(os.path.join(d, f"journal_{n}.log")).read())
    ok = attendu.get(n[:2]) == statut
    ko += not ok
    print(f"| `{n}` | {statut}{'' if ok else ' **(attendu ' + str(attendu.get(n[:2])) + ')**'} | `{b.get('code', '')}` | {detail}{', ' + m.group(1) + ' ms' if m and statut == 200 else ''} |")
sys.exit(1 if ko else 0)
