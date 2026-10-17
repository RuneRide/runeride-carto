# Données OpenStreetMap pré-traitées en local

Objectif : servir par nos propres API les stations-service (avec marque) et la recherche de localités, sans aucun traitement sur o2switch. Tout est calculé ici ; seul un dump SQL est envoyé plus tard (rien n'est déployé à ce jour).

**Attribution (ODbL)** : données © contributeurs OpenStreetMap, licence ODbL 1.0. Toute application ou API qui expose ces données doit afficher « © les contributeurs d'OpenStreetMap » avec un lien vers https://www.openstreetmap.org/copyright. Une base dérivée (ces tables) reste sous ODbL (partage à l'identique) si elle est redistribuée.

## Procédure reproductible

1. Télécharger https://download.geofabrik.de/europe/france-latest.osm.pbf et vérifier le MD5 publié (`.osm.pbf.md5`). Hors dépôt (jamais commité).
2. `bin/osm-extraire france.osm.pbf web/storage/osm/` (Python >= 3.9, `pip install osmium`). Produit `osm_dump.sql` et `stats.json`. `web/storage/*` est ignoré par git. Sans pyosmium local, on peut lancer le script dans un conteneur Python avec `osmium` installé (c'est ce qui a été fait : voir plus bas).
3. Créer la base et le schéma : `docs/osm/osm_schema.sql`, puis charger le dump en ligne de commande : `mariadb -u... -p runeride_osm < osm_dump.sql` (le dump est un INSERT par lots de 500 lignes, précédé de TRUNCATE, donc rejouable). Pour l'envoi : `gzip -k osm_dump.sql`.

Le schéma est dans `docs/osm/` et non dans `web/migrations/mariadb/` : c'est un jeu de données de référence rechargé en bloc, pas un état applicatif versionné. Le placer dans migrations/ le ferait appliquer automatiquement par `bin/migrer` (et entrerait dans sa numérotation, avec risque de collision avec les lots en cours). À intégrer en migration numérotée seulement le jour où l'on décide de l'embarquer.

Tables : `osm_stations` (id OSM, nom, marque, exploitant, carburants `fuel:*=yes`, adresse partielle, lat/lon DOUBLE, `geom` POINT SRID 4326 avec SPATIAL INDEX, index nom/marque) et `osm_lieux` (place=city|town|village|hamlet|suburb|quarter|neighbourhood, nom, nom_alt, population, lat/lon, geom, index nom). Le point est stocké `POINT(lon lat)`, compatible PostGIS.

## Mesures (France, exécutées le 17/10/2026)

Fichier : `france-261016.osm.pbf` (Geofabrik, Last-Modified 17/10/2026 00:03 UTC), 5 099 612 575 octets (4,75 Gio), MD5 `07162fcf35d330b63465601a029813cb` identique au MD5 publié. Téléchargement : environ 2 min 24 s sur la machine de l'auteur.

Machine : 20 coeurs, 61 Go de RAM, disque NVMe. Outil : pyosmium 4.3.1 (Python 3.12 dans un conteneur Docker ; `osmium-tool` absent, pas de venv possible sur l'hôte faute d'ensurepip).

| Mesure | Valeur |
|---|---|
| Filtrage + génération du dump (une passe, un seul coeur) | 103 s |
| Stations (amenity=fuel, noeuds + chemins) | 12 114 (dont 1 620 sans marque) |
| Localités (noeuds uniquement) | 300 515 : hamlet 231 469, village 37 333, neighbourhood 25 553, quarter 3 154, suburb 1 829, town 1 111, city 66 |
| Localités avec population | 36 768 |
| Adresses (addr:housenumber + addr:street), comptées seulement | 5 063 588 noeuds + 979 671 chemins = 6 043 259 |
| Dump SQL brut | 40 031 240 octets (38 Mio) |
| Dump gzip -9 | 9 045 494 octets (8,6 Mio) |
| Base sur disque (`du` du dossier MariaDB) | 103 Mo (données 32 Mo + index 25 Mo selon information_schema) |
| Import en ligne de commande dans MariaDB 11.8 (Docker) | 3,5 s |

Requêtes (base de test `runeride_osm`, MariaDB 11.8, caches chauds, durée via SHOW PROFILES ; une mesure, pas de moyenne) :

- Stations à moins de 10 km de Toulouse (boîte englobante `MBRContains` sur l'index spatial, puis `ST_Distance_Sphere` exact) : 38 résultats en 0,46 ms. EXPLAIN : `type=range`, `key=sx_osm_stations_geom`, 18 lignes examinées.
- Localité par préfixe (`nom LIKE 'Toulou%' ORDER BY population DESC LIMIT 10`) : 0,2 ms ; EXPLAIN `range` sur `ix_osm_lieux_nom` (11 lignes). `LIKE 'saint-g%'` (589 lignes) : 0,2 à 0,7 ms. La collation `utf8mb4_unicode_ci` rend la recherche insensible à la casse et aux accents.

## Ce qui n'a pas été testé

- Aucun chargement ni requête sur o2switch (version MariaDB, collation par défaut, `max_allowed_packet`, `ST_Distance_Sphere` et `REF_SYSTEM_ID` à vérifier sur leur version). Le schéma n'a été testé que sur MariaDB 11.8 ; le portage PostGIS est décrit en commentaire mais pas essayé.
- Les relations OSM (stations ou places cartographiées en relation) sont ignorées ; les `place=*` en chemin aussi. Les stations en chemin sont placées au centre de leurs points. Pas de dédoublonnage noeud/chemin pour une même station.
- Les adresses ne sont pas chargées : seul leur nombre est connu (6,0 M objets pour la France, hors relations).
- Pas de mesure pour l'Europe (aucune estimation faite).
- Pas de mise à jour incrémentale : on retélécharge et on rejoue le dump.

## Limites o2switch à confirmer

Ce sont celles données dans le brief, à reconfirmer dans les CGV et auprès du support avant de s'y fier : pas de programme serveur ni de binaire ; 420 s de CPU consécutives ; 500 000 inodes. Points à valider : taille maximale d'import acceptée (phpMyAdmin est limité, d'où l'import en ligne de commande par SSH `mariadb < dump.sql`, 3,5 s ici en local) ; le dump gzip de 8,6 Mio tient largement dans ces limites ; les 3 fichiers du dossier de travail et la base ne consomment que quelques inodes (la base InnoDB en occupe peu).

## Hors de portée du mutualisé (d'après ce qui a été mesuré ou non)

- Adresses : 6,0 M de lignes pour la France seulement ; ordre de grandeur à plusieurs fois les 300 k localités (environ 20 fois en nombre de lignes). Nous n'avons pas chargé ni mesuré les adresses, donc on ne sait pas si une recherche par préfixe sur ce volume tient dans les limites d'o2switch. La recherche plein texte tolérante aux fautes (trigrammes, classement) n'est pas testée et probablement mal adaptée à MariaDB sans moteur dédié.
- Routage : non testé, nécessite un moteur dédié (donc un programme serveur, interdit par les CGV) ; à traiter ailleurs.
