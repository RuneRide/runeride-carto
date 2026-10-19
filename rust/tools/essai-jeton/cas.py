# SPDX-License-Identifier: EUPL-1.2
# Écrit les événements Function URL (format API Gateway v2) des cas d'essai du jeton court, dans le dossier donné.
#   python3 -I cas.py <dossier contenant jetons.json>
import json, os, sys
d = sys.argv[1]
jetons = json.load(open(os.path.join(d, "jetons.json")))
dep = {"lat": 45.1885, "lon": 5.7245}          # Grenoble
lyon = {"lat": 45.764, "lon": 4.8357}
def ev(chemin, corps, jeton, methode="POST"):
    e = {"rawPath": chemin, "requestContext": {"http": {"method": methode}}, "headers": {"content-type": "application/json"},
         "body": json.dumps(corps) if corps is not None else "", "isBase64Encoded": False}
    if jeton:
        e["headers"]["authorization"] = "Bearer " + jeton
    return e
cas = {
    "01_sante_sans_jeton": ev("/v1/sante", None, None, "GET"),
    "02_balade_sans_jeton": ev("/v1/balade", {"depart": dep, "distance_km": 20}, None),
    "03_session_runeride": ev("/v1/balade", {"depart": dep, "distance_km": 20}, jetons["session_runeride"]),
    "04_expire": ev("/v1/balade", {"depart": dep, "distance_km": 20}, jetons["expire"]),
    "05_autre_cle": ev("/v1/balade", {"depart": dep, "distance_km": 20}, jetons["autre_cle"]),
    "06_gratuit_petite_20km": ev("/v1/balade", {"depart": dep, "distance_km": 20}, jetons["gratuit_petite_boucle"]),
    "07_gratuit_petite_40km": ev("/v1/balade", {"depart": dep, "distance_km": 40}, jetons["gratuit_petite_boucle"]),
    "08_gratuit_petite_180min": ev("/v1/balade", {"depart": dep, "duree_min": 180}, jetons["gratuit_petite_boucle"]),
    "09_gratuit_petite_trajet": ev("/v1/trajet", {"depart": dep, "arrivee": lyon}, jetons["gratuit_petite_boucle"]),
    "10_gratuit_grand_120km": ev("/v1/balade", {"depart": dep, "distance_km": 120}, jetons["gratuit_grand"]),
    "11_gratuit_grand_decouverte": ev("/v1/balade", {"depart": dep, "distance_km": 60, "preferences": {"decouverte": 3}}, jetons["gratuit_grand"]),
    "12_abonne_grand_decouverte": ev("/v1/balade", {"depart": dep, "distance_km": 60, "preferences": {"decouverte": 3}}, jetons["abonne_grand"]),
    "13_abonne_trajet": ev("/v1/trajet", {"depart": dep, "arrivee": lyon}, jetons["abonne_grand"]),
}
attendu = {"01": 200, "02": 401, "03": 401, "04": 401, "05": 401, "06": 200, "07": 403, "08": 403, "09": 403, "10": 200, "11": 403, "12": 200, "13": 200}
for n, e in cas.items():
    json.dump(e, open(os.path.join(d, f"evt_{n}.json"), "w"))
json.dump(attendu, open(os.path.join(d, "attendu.json"), "w"))
print(len(cas), "événements écrits")
