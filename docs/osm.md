# Données OpenStreetMap pré-traitées en local

Objectif : servir par nos propres API les stations-service (avec marque) et la recherche de localités, sans aucun traitement sur o2switch. Tout est calculé ici ; seuls des fichiers TSV et un script de chargement sont envoyés plus tard (rien n'est déployé à ce jour).

**Attribution (ODbL)** : données © contributeurs OpenStreetMap, licence ODbL 1.0. Toute application ou API qui expose ces données doit afficher « © les contributeurs d'OpenStreetMap » avec un lien vers https://www.openstreetmap.org/copyright. Une base dérivée (ces tables) reste sous ODbL (partage à l'identique) si elle est redistribuée.

## Procédure reproductible

1. Télécharger https://download.geofabrik.de/europe/france-latest.osm.pbf et vérifier le MD5 publié (`.osm.pbf.md5`). Hors dépôt (jamais commité).
2. `bin/osm-extraire france.osm.pbf dossier/` (Python >= 3.9, `pip install osmium`). Produit `osm_stations.tsv`, `osm_lieux.tsv`, `charger.sql` et `stats.json`. Hors dépôt (`*.tsv` et `web/storage/*` sont ignorés par git). Sans pyosmium local, on lance le script dans un conteneur Python (voir « Exécution » plus bas).
3. Créer la base et le schéma (`docs/osm/osm_schema.sql`), puis charger **depuis le dossier des TSV** : `cd dossier && mariadb --local-infile=1 -u... -p BASE < charger.sql` (TRUNCATE puis `LOAD DATA LOCAL INFILE` : rejouable). Pour l'envoi : `tar czf osm.tgz *.tsv charger.sql`.

## Import sûr (choix : TSV échappé + `LOAD DATA LOCAL INFILE`)

L'ancien import construisait des `INSERT` avec une fonction d'échappement écrite à la main (`sql()`), qui supprimait aussi les octets NUL. Il est remplacé : plus aucune valeur OSM n'est mise dans une chaîne SQL.

- `bin/carto_osm.py` écrit des fichiers TSV (séparateur tabulation, fin de ligne `\n`, `\N` pour NULL) en échappant `\`, tabulation, `\n`, `\r`, NUL (`\0`) et Ctrl-Z. Le texte « \N » est écrit `\\N` (donc pas pris pour NULL).
- `charger.sql` (généré, sans donnée) lit ces fichiers avec `LOAD DATA LOCAL INFILE ... CHARACTER SET utf8mb4 ... ESCAPED BY '\\'`; le point `geom` est construit par `ST_GeomFromText(CONCAT('POINT(', @lon, ' ', @lat, ')'))` à partir des deux nombres (aucun texte OSM).
- Pourquoi ce choix plutôt que des insertions paramétrées : fonctionne avec le seul client `mariadb` en SSH (aucun PHP ni Python à exécuter sur l'hébergeur), 300 k lignes en environ 3 s (contre 3,5 s pour l'ancien dump), pas de limite de paquet par `INSERT`. `mariadb-import --local` lirait les mêmes fichiers (non essayé) mais ne calcule pas `geom` : on garde `charger.sql`.
- Les textes plus longs que la colonne (255 caractères, 512 pour `nom_alt`, 32 pour les numéros et codes postaux) sont tronqués par le script, par caractères, et comptés dans `stats.json` (`tronques`, 0 sur la France) : un `LOAD DATA` strict échouerait sinon.
- Prérequis côté serveur : `local_infile` activé (c'est le cas ici, MariaDB 11.8) et un client lancé avec `--local-infile=1`. **Non vérifié sur o2switch** : si `local_infile` y est désactivé, il faudra le demander au support ou passer par un autre chemin (par exemple un script PHP de chargement en insertions préparées, non écrit).

### Test du chargement avec des noms piégés

`web/tests/Integration/ImportSurTest.php` génère un jeu piégé (`python3 bin/carto_osm.py pieges DIR`), le charge avec le vrai `charger.sql` dans une base jetable `carto_test_import` (créée puis supprimée), puis relit chaque colonne et compare octet pour octet : apostrophes, antislash (dont `\'`, `\\`, `\0`, `\n` littéraux), `'); DROP TABLE x;--`, `" OR 1=1 -- ; DROP TABLE osm_lieux; #`, octets NUL, emoji et caractères hors plan multilingue, retours à la ligne (`\n`, `\r\n`, `\r`), tabulations, Ctrl-Z, le texte `\N` et `NULL`, chaîne vide (reste vide, pas NULL), 255 caractères (conservés tels quels) et 5000 caractères (tronqués à 255 sans erreur ni débordement sur la ligne voisine). Une table sentinelle `x` doit survivre, `SHOW WARNINGS` doit être vide, et l'API `/v1/lieux` doit relire à l'identique quatre noms piégés (apostrophes, injection, antislash, emoji ; comparaison du JSON). Test Python sans base : `python3 -m unittest discover -s bin -p 'test_*.py'`.

Commande complète (base de test MariaDB avec un compte administrateur, `python3` et `mariadb` dans le PATH) :

```
cd web && CARTO_TEST_DSN='mysql:host=127.0.0.1;port=3307;dbname=runeride_osm;charset=utf8mb4' CARTO_TEST_USER=carto_ro CARTO_TEST_PASSWORD=carto_ro \
  CARTO_TEST_ADMIN_HOST=127.0.0.1 CARTO_TEST_ADMIN_PORT=3307 CARTO_TEST_ADMIN_USER=root CARTO_TEST_ADMIN_PASSWORD=root vendor/bin/phpunit
```

Sans les variables `CARTO_TEST_*`, les tests d'intégration sont ignorés (28 sur 72).

Le schéma est dans `docs/osm/` et non dans `web/migrations/mariadb/` : c'est un jeu de données de référence rechargé en bloc, pas un état applicatif versionné. Le placer dans migrations/ le ferait appliquer automatiquement par `bin/migrer` (et entrerait dans sa numérotation, avec risque de collision avec les lots en cours). À intégrer en migration numérotée seulement le jour où l'on décide de l'embarquer.

Tables : `osm_stations` (id OSM, nom, marque, exploitant, carburants `fuel:*=yes`, adresse partielle, lat/lon DOUBLE, `geom` POINT SRID 4326 avec SPATIAL INDEX, `min_zoom` TINYINT UNSIGNED, index nom/marque/min_zoom) et `osm_lieux` (place=city|town|village|hamlet|suburb|quarter|neighbourhood, nom, nom_alt, population, lat/lon, geom, `min_zoom`, index nom/min_zoom). Le point est stocké `POINT(lon lat)`, compatible PostGIS.

## `min_zoom` : règles et écrémage

Chaque point reçoit le zoom minimal (0 à 15) à partir duquel une carte doit l'afficher. Les règles sont des constantes en tête de `bin/carto_osm.py` (seuils, liste des grandes enseignes, taille des cellules d'écrémage) ; les modifier puis relancer l'extraction.

**Zoom de base** :

| Lieux | Règle |
|---|---|
| city | population >= 1 M : 0 ; >= 300 k : 2 ; >= 100 k : 3 ; sinon (ou sans population) : 4 |
| town | >= 20 k : 6 ; >= 5 k : 7 ; sinon 8 |
| village | >= 2 000 : 9 ; >= 500 : 10 ; sinon 11 |
| suburb : 12 ; hamlet, quarter : 13 ; neighbourhood : 14 | |

Stations : grande enseigne reconnue dans la marque ou le nom (TotalEnergies, Esso, BP, Shell, Avia, Leclerc, Carrefour, Intermarché, Auchan, Système U, Casino, Cora, Netto, Agip, Eni, Elan, Dyneff... voir la liste, comparaison par mots entiers, sans casse ni accents) : 10 ; autre marque renseignée : 11 ; sans marque : 12.

**Écrémage par grille** : à chaque zoom z <= 14, la carte est découpée en cellules de 1/4 de tuile de côté (lieux, environ 64 px) ou 1/8 de tuile (stations, environ 32 px). Les points sont traités du plus au moins important (zoom de base croissant, puis population décroissante pour les lieux, nombre de carburants décroissant pour les stations, puis ordre d'identifiant) ; chacun reçoit le plus petit zoom >= son zoom de base où sa cellule ne contient pas déjà un point plus important visible. Un point visible à z l'est à tous les zooms supérieurs. Si la cellule est encore prise à z14, le point passe à 15 ; à z15 tout est visible. Les cellules sont emboîtées (cellule de z+1 incluse dans celle de z), ce qui est vérifié par `bin/test_carto_osm.py` (au plus un point visible par cellule à chaque zoom, sur 3 000 points aléatoires).

Résultat sur la France (17/10/2026) : lieux visibles par zoom (cumul) : z0 1, z2 2, z3 4, z4 9, z5 20, z6 62, z7 207, z8 450, z9 1 813, z10 8 054, z11 28 114, z12 37 833, z13 183 533, z14 260 824, z15 300 515. Stations (aucune avant z10) : z10 6 167, z11 8 247, z12 10 628, z13 11 314, z14 11 646, z15 12 114. Exemple : Toulouse (504 078 hab., base 2) tombe à `min_zoom` 5 car des villes plus peuplées occupent déjà ses cellules aux zooms 2 à 4. C'est une conséquence assumée de l'écrémage, pas une erreur.

Pas évalué : la qualité visuelle de ces règles sur une vraie carte (seuils choisis a priori, jamais confrontés à un rendu), ni pour d'autres pays.

## Comparaison mesurée : index spatial, index `min_zoom`, clé de cellule

Question : « points visibles dans la boîte à zoom z » (`min_zoom <= z` dans la boîte). Base France chargée (300 515 lieux, 12 114 stations), MariaDB 11.8 en Docker, `innodb_buffer_pool_size` 128 Mio (données + index : environ 60 Mo pour les lieux, 5 Mo pour les stations), mesures faites **caches chauds** : une exécution d'échauffement par requête, puis 30 répétitions chronométrées côté PHP (PDO, requêtes préparées, connexion TCP locale), médiane par requête ; le tableau donne la médiane sur 5 centres (Paris, Toulouse, Lozère, Bretagne, Alpes). « Lignes » = `r_rows` de `ANALYZE` (lignes réellement lues dans la table ou l'index, médiane). Script : `php bin/bench-zoom.php 30`.

Variantes : **(a)** index spatial seul (`MBRContains` sur `geom`) puis filtre `min_zoom` ; **(d)** index sur `min_zoom` seul, puis filtre `lat`/`lon` ; **(b)** clé de cellule entière (tuile Web Mercator niveau 12, `x*4096+y`), un intervalle de clés par colonne de tuiles, index `(cell, min_zoom)`, puis filtre exact `lat`/`lon` ; **(c)** idem avec l'index `(min_zoom, cell)`. Pour (b) et (c), une colonne `cell` et deux index ont été ajoutés temporairement ; ils ne sont **pas** dans le schéma livré. Pour les recréer : `ALTER TABLE t ADD cell INT UNSIGNED NOT NULL DEFAULT 0; UPDATE t SET cell = (FLOOR((lon+180)/360*4096) << 12) | FLOOR((1 - LN(TAN(RADIANS(lat)) + 1/COS(RADIANS(lat)))/PI())/2*4096); ALTER TABLE t ADD INDEX ix_t_cell (cell, min_zoom), ADD INDEX ix_t_zoom_cell (min_zoom, cell);` (formule SQL non recoupée avec les clés calculées en PHP/Python ; en revanche, les variantes (a) (b) (c) (d) ont renvoyé le même nombre de résultats pour chaque boîte mesurée) ; `bin/bench-zoom.php` détecte alors la colonne et mesure (b) et (c).

Lieux (extrait ; temps médian en ms / lignes lues) :

| z | boîte (deg) | résultats | (a) spatial | (d) min_zoom | (b) cellule | (c) zoom+cellule |
|---|---|---|---|---|---|---|
| 4 | 12 x 8 | 5 | 168 / 209 499 | **0,035** / 9 | 491 / 5 | 0,20 / 5 |
| 4 | 30 x 20 | 9 | 245 / 300 515 | **0,035** / 9 | 1 714 / 9 | 0,45 / 9 |
| 8 | 4 x 3 | 59 | 28,7 / 34 541 | **0,37** / 450 | 30,6 / 64 | 0,81 / 64 |
| 9 | 2 x 1,4 | 67 | 7,1 / 8 123 | **1,36** / 1 813 | non mesuré | non mesuré |
| 10 | 1,2 x 0,8 | 128 | **2,6** / 3 128 | 5,6 / 8 054 | 1,34 / 139 | 3,8 / 139 |
| 11 | 0,6 x 0,4 | 79 | **0,79** / 875 | 19,4 / 28 114 | 0,44 / 110 | 6,7 / 110 |
| 12 | 0,3 x 0,2 | 21 | **0,35** / 376 | 25,8 / 37 833 | 0,20 / 36 | 7,0 / 36 |
| 15 | 0,06 x 0,04 | 34 | **0,064** / 34 | 205 / 300 515 | 0,13 / 117 | 33 / 117 |

Stations : mêmes conclusions en plus petit (aucune station avant z10) : (a) 5,7 à 8,6 ms à z4 contre 0,03 ms pour (d) ; à z10 à z15, (a) 0,03 à 0,16 ms, (d) 4 à 8 ms, (b) 0,03 à 0,2 ms.

Lecture :

- L'index spatial seul (a) ne profite pas de `min_zoom` : il lit toute la boîte puis jette. À z4 il lit toute la France (300 k lignes, 170 à 245 ms). Il est en revanche le meilleur ou à égalité à partir de z10 (lieux : 0,03 à 2,6 ms).
- L'index sur `min_zoom` (d) lit **tous** les points de `min_zoom <= z` quelle que soit la boîte (450 lieux à z8, 1 813 à z9, 8 054 à z10) : excellent tant que ce nombre est petit, mauvais dès que z >= 10 (5 à 205 ms).
- La clé de cellule (b) n'est jamais nettement meilleure que le meilleur des deux autres : elle gagne un facteur 1,5 à 2 sur (a) à z10 à z12 (en valeur absolue 0,15 à 1,3 ms), perd à z15 (0,13 contre 0,064 ms), et devient catastrophique à zoom bas parce qu'une grande boîte produit des centaines d'intervalles (1,7 s à z4 pour 30 x 20 degrés, 341 intervalles). (c) corrige le zoom bas mais multiplie le coût à zoom haut (33 ms à z15).
- Les écarts à z12 et plus sont de l'ordre de 0,1 ms, donc sous le bruit d'un appel PHP mutualisé.

**Recommandation retenue** (limitée à ces mesures) : pas de colonne `cell`. Un index spatial et un index sur `min_zoom`, et une stratégie par zoom, dans `Spatial::visibles()` : zoom <= 9 : index `min_zoom` puis filtre `lat`/`lon` ; zoom >= 10 : index spatial puis filtre `min_zoom`. Le seuil 9/10 vient des mesures ci-dessus (à z9 les deux sont du même ordre : 1,0 à 7,1 ms pour (a), 1,3 ms pour (d) ; à z10 (a) gagne). Il dépend du nombre de points par zoom, donc de l'écrémage et du jeu de données : à revoir si les règles ou le périmètre changent. L'écrémage borne aussi la taille d'une réponse par construction (au plus quelques dizaines de points par écran et par zoom).

Non mesuré : charges concurrentes, caches froids, buffer pool plus petit que les données (un mutualisé peut en avoir un autre), requêtes avec le `ORDER BY` / `LIMIT` de l'API (les mesures sont celles des requêtes brutes sans tri), MariaDB 10.6, PostGIS. Les `FORCE INDEX` sont spécifiques à MariaDB/MySQL et isolés dans `Spatial.php`.

## Mesures (France, exécutées le 17/10/2026)

Fichier : `france-261016.osm.pbf` (Geofabrik, Last-Modified 17/10/2026 00:03 UTC), 5 099 612 575 octets (4,75 Gio), MD5 `07162fcf35d330b63465601a029813cb` identique au MD5 publié. Téléchargement : environ 2 min 24 s sur la machine de l'auteur.

Les mesures de volume de ce tableau (PBF, filtrage, comptages) datent de la première version, refaites à l'identique avec la nouvelle (mêmes comptes : 12 114 stations, 300 515 lieux). Machine : 20 coeurs, 61 Go de RAM, disque NVMe. Outil : pyosmium 4.3.1 (Python 3.12 dans un conteneur Docker ; `osmium-tool` absent, pas de venv possible sur l'hôte faute d'ensurepip).

| Mesure | Valeur |
|---|---|
| Filtrage + génération des fichiers (une passe, un seul coeur) | 101 s (dont 2 s pour `min_zoom`, écrémage et écriture) |
| Stations (amenity=fuel, noeuds + chemins) | 12 114 (dont 1 620 sans marque) |
| Localités (noeuds uniquement) | 300 515 : hamlet 231 469, village 37 333, neighbourhood 25 553, quarter 3 154, suburb 1 829, town 1 111, city 66 |
| Localités avec population | 36 768 |
| Adresses (addr:housenumber + addr:street), comptées seulement | 5 063 588 noeuds + 979 671 chemins = 6 043 259 |
| TSV (lieux 21,6 Mo + stations 1,4 Mo) | environ 23 Mo, non comparé après compression |
| Base (information_schema, après `ANALYZE TABLE`) | lieux : données 31 Mo + index 30 Mo ; stations : 2,6 Mo + 2,4 Mo |
| Chargement `charger.sql` dans MariaDB 11.8 (Docker) | 2,9 s |

Requêtes de la première version (avant `min_zoom` ; base de test `runeride_osm`, MariaDB 11.8, caches chauds, durée via SHOW PROFILES ; une mesure, pas de moyenne) :

- Stations à moins de 10 km de Toulouse (boîte englobante `MBRContains` sur l'index spatial, puis `ST_Distance_Sphere` exact) : 38 résultats en 0,46 ms. EXPLAIN : `type=range`, `key=sx_osm_stations_geom`, 18 lignes examinées.
- Localité par préfixe (`nom LIKE 'Toulou%' ORDER BY population DESC LIMIT 10`) : 0,2 ms ; EXPLAIN `range` sur `ix_osm_lieux_nom` (11 lignes). `LIKE 'saint-g%'` (589 lignes) : 0,2 à 0,7 ms. La collation `utf8mb4_unicode_ci` rend la recherche insensible à la casse et aux accents.

## Exécution (conteneur, sans venv ni sudo)

```
pip install --target LIB osmium   # une fois, dans un conteneur mcr.microsoft.com/playwright/python:v1.58.0-noble
docker run --rm -u $(id -u):$(id -g) -v $PWD/bin:/bin2:ro -v SCRATCH:/s -e PYTHONPATH=/s/work/lib -e PYTHONDONTWRITEBYTECODE=1 \
  mcr.microsoft.com/playwright/python:v1.58.0-noble python3 /bin2/osm-extraire /s/pbf/france.osm.pbf /s/out
```

## Ce qui n'a pas été testé

- La qualité cartographique des règles de `min_zoom` (aucun rendu), et le chargement des TSV avec un `local_infile` désactivé.
- Aucun chargement ni requête sur o2switch (version MariaDB, `local_infile`, collation par défaut, `ST_Distance_Sphere` et `REF_SYSTEM_ID` à vérifier sur leur version). Le schéma n'a été testé que sur MariaDB 11.8 ; le portage PostGIS est décrit en commentaire mais pas essayé.
- Les relations OSM (stations ou places cartographiées en relation) sont ignorées ; les `place=*` en chemin aussi. Les stations en chemin sont placées au centre de leurs points. Pas de dédoublonnage noeud/chemin pour une même station.
- Les adresses ne sont pas chargées : seul leur nombre est connu (6,0 M objets pour la France, hors relations).
- Pas de mesure pour l'Europe (aucune estimation faite).
- Pas de mise à jour incrémentale : on retélécharge et on rejoue le chargement.

## Limites o2switch à confirmer

Ce sont celles données dans le brief, à reconfirmer dans les CGV et auprès du support avant de s'y fier : pas de programme serveur ni de binaire ; 420 s de CPU consécutives ; 500 000 inodes. Points à valider : taille maximale d'import acceptée (phpMyAdmin est limité, d'où l'import en ligne de commande par SSH `mariadb --local-infile=1 < charger.sql`, 2,9 s ici en local) ; les ~23 Mo de TSV tiennent largement dans ces limites ; les fichiers du dossier de travail et la base ne consomment que quelques inodes (la base InnoDB en occupe peu).

## Hors de portée du mutualisé (d'après ce qui a été mesuré ou non)

- Adresses : 6,0 M de lignes pour la France seulement ; ordre de grandeur à plusieurs fois les 300 k localités (environ 20 fois en nombre de lignes). Nous n'avons pas chargé ni mesuré les adresses, donc on ne sait pas si une recherche par préfixe sur ce volume tient dans les limites d'o2switch. La recherche plein texte tolérante aux fautes (trigrammes, classement) n'est pas testée et probablement mal adaptée à MariaDB sans moteur dédié.
- Routage : non testé, nécessite un moteur dédié (donc un programme serveur, interdit par les CGV) ; à traiter ailleurs.
