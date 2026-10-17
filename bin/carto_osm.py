"""Bibliothèque de l'import OSM de RuneRide Carto (sans dépendance, testable seule).

- Écriture TSV avec échappement `LOAD DATA` (aucun SQL construit à la main) ;
- règles de `min_zoom` (configurables ci-dessous) et écrémage par grille ;

Utilisée par `bin/osm-extraire` (qui seul a besoin de pyosmium) et par les tests.
Données © contributeurs OpenStreetMap, licence ODbL.
"""
import json
import math
import os
import sys
import unicodedata

# ---------------------------------------------------------------- configuration
# Tout ce qui se règle est ici ; les valeurs sont documentées dans docs/osm.md.

ZOOM_MAX = 15            # min_zoom maximal : à partir de z15 tout est visible (tuiles z15)
ZOOM_ECREMAGE_MAX = 14   # on écrème aux zooms <= 14 ; un point encore masqué à z14 passe à 15

# Taille de la cellule d'écrémage : 2**K cellules par côté de tuile de 256 px à chaque zoom
# (K=2 -> cellule ~64 px). Une cellule garde au plus UN point à un zoom donné.
ECREMAGE_K = {"lieux": 2, "stations": 3}  # stations : icônes plus petites (~32 px)

# Lieux : (seuil de population minimal, min_zoom de base), du plus grand au plus petit seuil ;
# le dernier couple (0, z) sert aussi aux lieux sans population renseignée.
LIEUX_BASE = {
    "city": [(1_000_000, 0), (300_000, 2), (100_000, 3), (0, 4)],
    "town": [(20_000, 6), (5_000, 7), (0, 8)],
    "village": [(2_000, 9), (500, 10), (0, 11)],
    "suburb": [(0, 12)],
    "hamlet": [(0, 13)],
    "quarter": [(0, 13)],
    "neighbourhood": [(0, 14)],
}

# Stations : grandes enseignes (comparaison sur le nom ou la marque, sans accents ni casse).
MARQUES_MAJEURES = (
    "total", "totalenergies", "esso", "bp", "shell", "avia", "leclerc", "carrefour", "intermarche", "auchan",
    "super u", "hyper u", "systeme u", "u express", "casino", "cora", "netto", "agip", "eni",
    "elan", "dyneff", "esso express", "vito", "ecomarche", "leader price", "match", "colruyt",
)
STATION_ZOOM_MAJEURE = 10   # enseigne connue
STATION_ZOOM_MARQUE = 11    # autre marque renseignée
STATION_ZOOM_SANS_MARQUE = 12

# Longueurs des colonnes VARCHAR (en caractères) : plus long = tronqué et compté.
LONGUEURS = {"nom": 255, "marque": 255, "exploitant": 255, "carburants": 255, "adr_numero": 32,
             "adr_rue": 255, "adr_cp": 32, "adr_ville": 255, "nom_alt": 512}

COLONNES_STATIONS = ["osm_type", "osm_id", "nom", "marque", "exploitant", "carburants", "adr_numero",
                     "adr_rue", "adr_cp", "adr_ville", "lat", "lon", "min_zoom"]
COLONNES_LIEUX = ["osm_type", "osm_id", "categorie", "nom", "nom_alt", "population", "lat", "lon",
                  "min_zoom"]

# ---------------------------------------------------------------- TSV sûr


def sans_substituts(s):
    """Remplace les caractères non encodables en UTF-8 (substituts isolés) pour ne jamais écrire d'octets invalides."""
    return s.encode("utf-8", "replace").decode("utf-8")


def tronquer(s, longueur, compteur=None):
    """Tronque à `longueur` caractères (code points) ; compte les troncatures dans `compteur` si fourni."""
    if len(s) > longueur:
        if compteur is not None:
            compteur["tronques"] = compteur.get("tronques", 0) + 1
        return s[:longueur]
    return s


_ECHAPPEMENT = str.maketrans({"\\": "\\\\", "\t": "\\t", "\n": "\\n", "\r": "\\r", "\0": "\\0", "\x1a": "\\Z"})


def champ(v):
    """Une valeur -> texte d'un champ LOAD DATA (ESCAPED BY '\\', TERMINATED BY tab, LINES BY \\n)."""
    if v is None:
        return "\\N"
    if isinstance(v, bool):
        raise TypeError("booléen non géré")
    if isinstance(v, (int, float)):
        return repr(v)
    return sans_substituts(v).translate(_ECHAPPEMENT)


def ligne(valeurs):
    return "\t".join(champ(v) for v in valeurs) + "\n"


# ---------------------------------------------------------------- géométrie


def tuile(lat, lon, z):
    """Tuile Web Mercator (x, y) au niveau z (bornes de latitude ±85,0511)."""
    n = 1 << z
    lat = max(-85.0511287798, min(85.0511287798, lat))
    lon = max(-180.0, min(180.0, lon))
    x = int((lon + 180.0) / 360.0 * n)
    y = int((1.0 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2.0 * n)
    return min(max(x, 0), n - 1), min(max(y, 0), n - 1)


# ---------------------------------------------------------------- min_zoom


def _sans_accents(s):
    return "".join(c for c in unicodedata.normalize("NFD", s.lower()) if unicodedata.category(c) != "Mn")


def zoom_base_lieu(categorie, population, regles=LIEUX_BASE):
    for seuil, z in regles.get(categorie, [(0, ZOOM_MAX)]):
        if (population or 0) >= seuil:
            return z
    return ZOOM_MAX


def marque_majeure(nom, marque, liste=MARQUES_MAJEURES):
    texte = " " + _sans_accents((marque or "") + " " + (nom or "")) + " "
    texte = "".join(c if c.isalnum() else " " for c in texte)
    texte = " " + " ".join(texte.split()) + " "
    return any(" " + m + " " in texte for m in liste)


def zoom_base_station(nom, marque):
    if marque_majeure(nom, marque):
        return STATION_ZOOM_MAJEURE
    return STATION_ZOOM_MARQUE if marque else STATION_ZOOM_SANS_MARQUE


def ecremer(points, k, zmax_ecremage=ZOOM_ECREMAGE_MAX, zoom_max=ZOOM_MAX):
    """Calcule le min_zoom final de chaque point.

    `points` : liste de (zoom_base, importance, lat, lon) ; importance plus grande = plus important.
    Pour chaque point, du plus au moins important (zoom_base croissant, puis importance décroissante,
    puis rang d'origine pour rester déterministe), on cherche le plus petit zoom z >= zoom_base tel que
    sa cellule (2**k cellules par côté de tuile à z) ne contienne encore aucun point déjà visible à z.
    Un point visible à z l'est à tous les zooms supérieurs. Au-delà de zmax_ecremage : zoom_max.
    Retourne la liste des min_zoom dans l'ordre d'entrée.
    """
    niveau_fin = zmax_ecremage + k
    xy = [tuile(p[2], p[3], niveau_fin) for p in points]
    ordre = sorted(range(len(points)), key=lambda i: (points[i][0], -points[i][1], i))
    occupe = [set() for _ in range(zmax_ecremage + 1)]
    res = [zoom_max] * len(points)
    for i in ordre:
        base = points[i][0]
        x, y = xy[i]
        choisi = zoom_max
        for z in range(min(base, zoom_max), zmax_ecremage + 1):
            s = niveau_fin - (z + k)
            if ((x >> s) << 32 | (y >> s)) not in occupe[z]:
                choisi = z
                break
        res[i] = choisi
        for z in range(choisi, zmax_ecremage + 1):
            s = niveau_fin - (z + k)
            occupe[z].add((x >> s) << 32 | (y >> s))
    return res


# ---------------------------------------------------------------- écriture des fichiers

CHARGER_SQL = """-- Chargement des fichiers TSV produits par osm-extraire (généré ; données © contributeurs OpenStreetMap, ODbL).
-- À lancer depuis CE dossier : mariadb --local-infile=1 -u... -p BASE < charger.sql
-- Les valeurs ne passent jamais par une chaîne SQL : LOAD DATA lit les fichiers (échappement TSV).
SET NAMES utf8mb4;
SET SESSION sql_mode = REPLACE(@@sql_mode, 'NO_BACKSLASH_ESCAPES', '');
SET SESSION unique_checks = 0, foreign_key_checks = 0;
TRUNCATE osm_stations;
LOAD DATA LOCAL INFILE 'osm_stations.tsv' INTO TABLE osm_stations CHARACTER SET utf8mb4
  FIELDS TERMINATED BY '\\t' ESCAPED BY '\\\\' LINES TERMINATED BY '\\n'
  (osm_type, osm_id, nom, marque, exploitant, carburants, adr_numero, adr_rue, adr_cp, adr_ville, @lat, @lon, min_zoom)
  SET lat = @lat, lon = @lon, geom = ST_GeomFromText(CONCAT('POINT(', @lon, ' ', @lat, ')'), 4326);
TRUNCATE osm_lieux;
LOAD DATA LOCAL INFILE 'osm_lieux.tsv' INTO TABLE osm_lieux CHARACTER SET utf8mb4
  FIELDS TERMINATED BY '\\t' ESCAPED BY '\\\\' LINES TERMINATED BY '\\n'
  (osm_type, osm_id, categorie, nom, nom_alt, population, @lat, @lon, min_zoom)
  SET lat = @lat, lon = @lon, geom = ST_GeomFromText(CONCAT('POINT(', @lon, ' ', @lat, ')'), 4326);
SHOW WARNINGS;
"""


def preparer_station(r, compteur=None):
    """r : dict de champs bruts -> liste de valeurs (sans min_zoom), textes tronqués."""
    v = [r.get(c) for c in COLONNES_STATIONS[:12]]
    for i, c in enumerate(COLONNES_STATIONS[:12]):
        if isinstance(v[i], str):
            v[i] = tronquer(sans_substituts(v[i]), LONGUEURS[c], compteur) if c in LONGUEURS else v[i]
    return v


def preparer_lieu(r, compteur=None):
    v = [r.get(c) for c in COLONNES_LIEUX[:8]]
    for i, c in enumerate(COLONNES_LIEUX[:8]):
        if isinstance(v[i], str) and c in LONGUEURS:
            v[i] = tronquer(sans_substituts(v[i]), LONGUEURS[c], compteur)
    return v


def ecrire(dossier, stations, lieux, compteur=None):
    """Écrit osm_stations.tsv, osm_lieux.tsv et charger.sql dans `dossier`.

    stations, lieux : listes de dicts (clés = colonnes sans min_zoom ; `carburants` déjà joint).
    Retourne un dict de statistiques (répartition des min_zoom, nombre de troncatures).
    """
    os.makedirs(dossier, exist_ok=True)
    compteur = compteur if compteur is not None else {}
    sl = [preparer_station(r, compteur) for r in stations]
    ll = [preparer_lieu(r, compteur) for r in lieux]
    zs = ecremer([(zoom_base_station(r.get("nom"), r.get("marque")),
                   len((r.get("carburants") or "").split(",")) if r.get("carburants") else 0, r["lat"], r["lon"])
                  for r in stations], ECREMAGE_K["stations"])
    zl = ecremer([(zoom_base_lieu(r["categorie"], r.get("population")), r.get("population") or 0, r["lat"], r["lon"])
                  for r in lieux], ECREMAGE_K["lieux"])
    stats = {"stations_min_zoom": {}, "lieux_min_zoom": {}, "tronques": compteur.get("tronques", 0)}
    with open(os.path.join(dossier, "osm_stations.tsv"), "w", encoding="utf-8", newline="") as f:
        for v, z in zip(sl, zs):
            f.write(ligne(v + [z]))
            stats["stations_min_zoom"][z] = stats["stations_min_zoom"].get(z, 0) + 1
    with open(os.path.join(dossier, "osm_lieux.tsv"), "w", encoding="utf-8", newline="") as f:
        for v, z in zip(ll, zl):
            f.write(ligne(v + [z]))
            stats["lieux_min_zoom"][z] = stats["lieux_min_zoom"].get(z, 0) + 1
    with open(os.path.join(dossier, "charger.sql"), "w", encoding="utf-8") as f:
        f.write(CHARGER_SQL)
    stats["stations_min_zoom"] = dict(sorted(stats["stations_min_zoom"].items()))
    stats["lieux_min_zoom"] = dict(sorted(stats["lieux_min_zoom"].items()))
    return stats


# ---------------------------------------------------------------- jeu piégé (tests)

CHAINES_PIEGES = [
    "simple",
    "L'apostrophe d'Anne",
    "anti\\slash \\' \\\\ \\0 \\n fin\\",
    "'); DROP TABLE x;--",
    "\" OR 1=1 -- ; DROP TABLE osm_lieux; #",
    "nul\0au\0milieu",
    "emoji \U0001F3CD\U0001F6E2 éè 中文 مرحبا",
    "ligne1\nligne2\r\nligne3\rfin",
    "tab\tulation\t\ttriple",
    "fin\x1a ctrl-Z et \x08 backspace",
    "\\N",
    "NULL",
    "",
    " espaces autour ",
    "x" * 255,
    "\U0001F3CD" * 255,
]


def jeu_pieges():
    """Lieux piégés (dicts) : mêmes noms en `nom`, plus un nom trop long (5000 car.) tronqué à 255."""
    lieux = []
    for i, nom in enumerate(CHAINES_PIEGES):
        lieux.append({"osm_type": "n", "osm_id": 9_000_000_000 + i, "categorie": "hamlet", "nom": nom,
                      "nom_alt": nom[::-1], "population": i, "lat": 43.0 + i / 1000, "lon": 1.0 + i / 1000})
    lieux.append({"osm_type": "n", "osm_id": 9_100_000_000, "categorie": "hamlet", "nom": "L" * 5000,
                  "nom_alt": None, "population": None, "lat": 44.0, "lon": 2.0})
    stations = [{"osm_type": "w", "osm_id": 9_200_000_000 + i, "nom": nom, "marque": nom, "exploitant": nom,
                 "carburants": None, "adr_numero": None, "adr_rue": nom, "adr_cp": None, "adr_ville": nom,
                 "lat": 45.0 + i / 1000, "lon": 3.0 + i / 1000} for i, nom in enumerate(CHAINES_PIEGES)]
    return stations, lieux


def attendu_json(stations, lieux):
    """Valeurs attendues en base (après troncature) : {table: {id: {colonne: valeur ou None}}} en JSON."""
    out = {"osm_lieux": {}, "osm_stations": {}}
    for r in lieux:
        p = preparer_lieu(r)
        out["osm_lieux"]["%s/%d" % (r["osm_type"], r["osm_id"])] = dict(zip(COLONNES_LIEUX, p))
    for r in stations:
        p = preparer_station(r)
        out["osm_stations"]["%s/%d" % (r["osm_type"], r["osm_id"])] = dict(zip(COLONNES_STATIONS, p))
    return json.dumps(out, ensure_ascii=True)


if __name__ == "__main__":
    # python3 carto_osm.py pieges DOSSIER : écrit le jeu piégé (TSV + charger.sql + attendu.json)
    if len(sys.argv) == 3 and sys.argv[1] == "pieges":
        s, l = jeu_pieges()
        ecrire(sys.argv[2], s, l)
        with open(os.path.join(sys.argv[2], "attendu.json"), "w") as fh:
            fh.write(attendu_json(s, l))
    else:
        sys.exit("usage : carto_osm.py pieges DOSSIER")
