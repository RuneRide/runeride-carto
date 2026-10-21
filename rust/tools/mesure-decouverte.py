# SPDX-License-Identifier: EUPL-1.2
# Mesure de l'écart de distance des boucles avec découverte (voir docs/raido.md §2.6) :
#   python3 -I mesure-decouverte.py <distance_km> <nombre de départs>
# Prérequis : bootstrap compilé en release et le graphe Rhône-Alpes (chemins en tête de fichier).
# Mesure : écart de distance des boucles avec découverte, sur plusieurs départs, avec l'historique d'une boucle précédente.
import json, os, subprocess, sys, random
B = '/home/philippe/github.com/runeride/runeride-carto/rust/target/release/bootstrap'
G = os.environ.get('RAIDO_GRAPHE', '/home/philippe/github.com/runeride/runeride-carto/donnees/graphe/rhone-alpes')
ENV = dict(os.environ, RAIDO_SOURCE=f'local:{G}', RAIDO_CACHE='', RAIDO_JETON='desactive', RAIDO_THREADS='4', RAIDO_CLE_SERVICE='k')
def run(chemin, corps, h=None):
    evt = {"rawPath": chemin, "requestContext": {"http": {"method": "POST"}}, "headers": h or {}, "body": json.dumps(corps), "isBase64Encoded": False}
    open(os.path.join(os.environ.get('TMPDIR', '/tmp'), 'raido-mesure-e.json'), 'w').write(json.dumps(evt))
    p = subprocess.run([B, '--evenement', os.path.join(os.environ.get('TMPDIR', '/tmp'), 'raido-mesure-e.json')], capture_output=True, text=True, env=ENV)
    r = json.loads(p.stdout); return r['statusCode'], json.loads(r['body'])
def dec(t):
    i = la = lo = 0; pts = []
    while i < len(t):
        o = []
        for _ in range(2):
            res = sh = 0
            while True:
                b = ord(t[i]) - 63; i += 1; res |= (b & 31) << sh; sh += 5
                if b < 32: break
            o.append(~(res >> 1) if res & 1 else res >> 1)
        la += o[0]; lo += o[1]; pts.append([la / 1e5, lo / 1e5])
    return pts
dist = float(sys.argv[1]) if len(sys.argv) > 1 else 25
n = int(sys.argv[2]) if len(sys.argv) > 2 else 10
random.seed(7)
bornes = json.load(open(f'{G}/index.json'))['bornes']
res = {k: [] for k in (0, 2, 3, 4)}; nouv = {k: [] for k in (0, 2, 3, 4)}; essais = 0
while len(res[4]) < n and essais < n * 5:
    essais += 1
    lat = bornes[0] + (bornes[2] - bornes[0]) * (0.2 + 0.6 * random.random()); lon = bornes[1] + (bornes[3] - bornes[1]) * (0.2 + 0.6 * random.random())
    base = {"depart": {"lat": lat, "lon": lon}, "retour": True, "distance_km": dist, "preferences": {"sinuosite": 3, "scenique": 2}}
    st, b = run('/v1/balade', base)
    if st != 200: continue
    st, a = run('/v1/apparier', {"trace": dec(b['trace'])}, {"x-cle-service": "k"})
    if st != 200 or not a['troncons']: continue
    ok = True; tmp = {}
    for d in (0, 2, 3, 4):
        st, r = run('/v1/balade', {**base, "preferences": {**base["preferences"], "decouverte": d}, "troncons_connus": a['troncons']})
        if st != 200: ok = False; break
        tmp[d] = ((r['distance_m'] / 1000 - dist) / dist * 100, r['scores']['nouveaute'] * 100)
    if not ok: continue
    for d in tmp: res[d].append(tmp[d][0]); nouv[d].append(tmp[d][1])
print(f"{len(res[4])} départs, boucles de {dist:.0f} km avec l'historique d'une boucle précédente")
print("découverte | écart moyen | |écart| moyen | |écart| max | hors ±15 % | nouveauté moyenne")
for d in (0, 2, 3, 4):
    e = res[d]; m = lambda v: sum(v) / len(v)
    print(f"{d:^10} | {m(e):+9.1f} % | {m([abs(x) for x in e]):11.1f} % | {max(abs(x) for x in e):9.1f} % | {sum(abs(x) > 15 for x in e):>4}/{len(e)}    | {m(nouv[d]):6.0f} %")
