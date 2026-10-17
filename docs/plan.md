# Plan RuneRide Carto

Dépôt séparé de `runeride` (baseline : comptes, trajets). Les outils OSM et le PoC de tuiles en viennent.

## Lots

1. **Socle PHP** : même structure que `runeride/web` (Config, Database, Router, migrations par pilote), base MariaDB distincte de celle des comptes.
2. **Import OSM** : `bin/osm-extraire` (fait, voir `docs/osm.md`). À ajouter : `min_zoom` par point, écrémage par grille, comparaison index spatial / clé de cellule.
3. **API de données** (interne) : stations autour d'un point (avec marque), recherche de lieux par préfixe, filtrage par zoom. Contrat versionné `/v1/…`. SQL isolé dans des dépôts de classes, portable vers PostgreSQL/PostGIS.
4. **Tuiles** : fichiers PMTiles statiques (France z15, Europe bas zoom), servis par Apache ; voir `docs/tuiles.md`. Polices et sprites à auto-héberger.
5. **Brancher RuneRide** : `FOND_DE_CARTE` et les routes de ravitaillement de la baseline pointent vers cette API.
6. **Plus tard, hors mutualisé (AWS)** : adresses complètes, itinéraires.

## À confirmer

- Version de MariaDB réelle chez o2switch (testé en local : 10.6 et 11.8), `max_allowed_packet`, quotas disque/IO.
- Limites des CGV pour des fichiers de plusieurs Go (interdit au-delà de 50 Go par fichier) et pour un service public.
- Aucun accès tiers à l'API sans accord d'o2switch.
