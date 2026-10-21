#!/usr/bin/env python3
# SPDX-License-Identifier: EUPL-1.2
"""Calibration du score scénique (jalon J2, spec §10) : les balades de référence sortent-elles parmi les 3 premiers candidats ?

Pour chaque fichier GPX d'un dossier (une trace par fichier, première trace du fichier), l'outil prend le premier point comme
départ et la longueur de la trace comme distance cible, demande à Raido les 3 premières variantes (boucle, `variante` 0 à 2), et
compare chaque variante à la référence : **couverture** = part de la longueur de la référence qui passe à moins de `--rayon` mètres
de la variante (100 m par défaut). La référence « sort » si une variante atteint `--seuil` de couverture (70 % par défaut).
Critère de la spec : au moins 7 références sur 10.

    python3 -I evaluer-references.py DOSSIER_GPX GRAPHE [--poids a.json b.json ...] [--sinuosite 3 --scenique 4 --motard 2]

Avec `--poids`, chaque fichier de poids reconstruit un graphe (≈ 9 s sur Rhône-Alpes, `raido-graphe construire --poids`) et
affiche une ligne du tableau ; sans, le graphe donné est évalué tel quel. Prérequis : `bootstrap` et `raido-graphe` en release,
le PBF (`--pbf`, défaut donnees/pbf/rhone-alpes-latest.osm.pbf). Rien n'est envoyé nulle part.
"""
import argparse, glob, json, math, os, subprocess, sys, tempfile
import xml.etree.ElementTree as ET

RACINE = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
BOOTSTRAP = os.path.join(RACINE, 'rust', 'target', 'release', 'bootstrap')
CONSTRUIRE = os.path.join(RACINE, 'rust', 'target', 'release', 'raido-graphe')


def lire_gpx(chemin):
    """Points (lat, lon) de la première trace (trkpt, sinon rtept) d'un fichier GPX."""
    racine = ET.parse(chemin).getroot()
    pts = [(float(e.get('lat')), float(e.get('lon'))) for e in racine.iter() if e.tag.split('}')[-1] == 'trkpt']
    return pts or [(float(e.get('lat')), float(e.get('lon'))) for e in racine.iter() if e.tag.split('}')[-1] == 'rtept']


def metres(a, b):
    return math.hypot((a[0] - b[0]) * 110540.0, (a[1] - b[1]) * 111320.0 * math.cos(math.radians((a[0] + b[0]) / 2)))


def longueur(pts):
    return sum(metres(a, b) for a, b in zip(pts, pts[1:]))


def reechantillonner(pts, pas):
    """Points tous les `pas` mètres le long de la polyligne (premier et dernier inclus)."""
    sortie, reste = [pts[0]], 0.0
    for a, b in zip(pts, pts[1:]):
        d = metres(a, b)
        t = pas - reste
        while t <= d:
            f = t / d if d else 0
            sortie.append((a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f))
            t += pas
        reste = (reste + d) % pas if d < pas - reste else d - (t - pas)
    sortie.append(pts[-1])
    return sortie


def couverture(reference, candidat, rayon):
    """Part (0..1) de la longueur de `reference` à moins de `rayon` m de `candidat`."""
    cellule = rayon / 100000.0
    grille = {}
    for p in reechantillonner(candidat, rayon / 2):
        grille.setdefault((int(p[0] / cellule), int(p[1] / cellule)), []).append(p)
    ech = reechantillonner(reference, 25)
    proches = 0
    for p in ech:
        i, j = int(p[0] / cellule), int(p[1] / cellule)
        if any(metres(p, q) <= rayon for di in (-2, -1, 0, 1, 2) for dj in (-2, -1, 0, 1, 2) for q in grille.get((i + di, j + dj), ())):
            proches += 1
    return proches / len(ech)


def decoder(t):
    i = la = lo = 0
    pts = []
    while i < len(t):
        o = []
        for _ in range(2):
            res = sh = 0
            while True:
                b = ord(t[i]) - 63
                i += 1
                res |= (b & 31) << sh
                sh += 5
                if b < 32:
                    break
            o.append(~(res >> 1) if res & 1 else res >> 1)
        la += o[0]
        lo += o[1]
        pts.append((la / 1e5, lo / 1e5))
    return pts


def variantes(graphe, depart, distance_km, prefs, n=3):
    env = dict(os.environ, RAIDO_SOURCE=f'local:{graphe}', RAIDO_CACHE='', RAIDO_JETON='desactive', RAIDO_THREADS='4')
    sortie = []
    for v in range(n):
        corps = {"depart": {"lat": depart[0], "lon": depart[1]}, "retour": True, "distance_km": distance_km, "variante": v, "preferences": prefs}
        evt = {"rawPath": "/v1/balade", "requestContext": {"http": {"method": "POST"}}, "headers": {}, "body": json.dumps(corps), "isBase64Encoded": False}
        with tempfile.NamedTemporaryFile('w', suffix='.json', delete=False) as f:
            json.dump(evt, f)
        r = json.loads(subprocess.run([BOOTSTRAP, '--evenement', f.name], capture_output=True, text=True, env=env).stdout)
        os.unlink(f.name)
        if r['statusCode'] == 200:
            sortie.append(decoder(json.loads(r['body'])['trace']))
    return sortie


def evaluer(graphe, refs, prefs, rayon, seuil):
    resultats = []
    for nom, pts in refs:
        dist = min(600.0, max(2.0, longueur(pts) / 1000.0))
        cands = variantes(graphe, pts[0], dist, prefs)
        meilleur = max((couverture(pts, c, rayon) for c in cands), default=0.0)
        resultats.append((nom, dist, meilleur, meilleur >= seuil))
    return resultats


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('dossier')
    ap.add_argument('graphe')
    ap.add_argument('--poids', nargs='*', default=[])
    ap.add_argument('--pbf', default=os.path.join(RACINE, 'donnees', 'pbf', 'rhone-alpes-latest.osm.pbf'))
    ap.add_argument('--sinuosite', type=int, default=3)
    ap.add_argument('--scenique', type=int, default=3)
    ap.add_argument('--motard', type=int, default=2)
    ap.add_argument('--rayon', type=float, default=100.0)
    ap.add_argument('--seuil', type=float, default=0.70)
    a = ap.parse_args()
    fichiers = sorted(glob.glob(os.path.join(a.dossier, '*.gpx')))
    if not fichiers:
        sys.exit(f'aucun fichier .gpx dans {a.dossier}')
    refs = [(os.path.basename(f), lire_gpx(f)) for f in fichiers]
    refs = [(n, p) for n, p in refs if len(p) >= 2]
    prefs = {"sinuosite": a.sinuosite, "scenique": a.scenique, "motard": a.motard, "agglo": 3}
    jeux = [(p, None) for p in a.poids] or [(None, a.graphe)]
    print(f"{len(refs)} références ; rayon {a.rayon:.0f} m, seuil {a.seuil:.0%}, curseurs {prefs}")
    for poids, graphe in jeux:
        if poids:
            graphe = tempfile.mkdtemp(prefix='raido-calibration-')
            r = subprocess.run([CONSTRUIRE, 'construire', a.pbf, graphe, '--poids', poids], capture_output=True, text=True)
            if r.returncode != 0:
                sys.exit(f"construction du graphe avec {poids} impossible :\n{r.stderr.strip().splitlines()[-1] if r.stderr.strip() else r.returncode}")
        res = evaluer(graphe, refs, prefs, a.rayon, a.seuil)
        ok = sum(1 for r in res if r[3])
        print(f"\n== {poids or graphe} : {ok}/{len(res)} références retrouvées (critère de la spec : 7 sur 10)")
        for nom, dist, cov, bon in res:
            print(f"  {'OK ' if bon else 'KO '} {nom:<32} {dist:6.1f} km  couverture {cov:4.0%}")


if __name__ == '__main__':
    main()
