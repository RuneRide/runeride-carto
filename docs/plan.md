# Plan RuneRide Carto

Dépôt séparé de `runeride` (baseline : comptes, trajets). Les outils OSM et le PoC de tuiles en viennent.

## Lots

1. **Socle PHP** (fait, `web/`) : même structure que `runeride/web` (Config, Database, Router, migrations par pilote), base MariaDB distincte de celle des comptes.
2. **Import OSM** : `bin/osm-extraire` (fait, voir `docs/osm.md`). À ajouter : `min_zoom` par point, écrémage par grille, comparaison index spatial / clé de cellule.
3. **API de données** (interne, fait sauf filtrage par zoom ; voir `docs/api.md`) : stations autour d'un point (avec marque), recherche de lieux par préfixe, filtrage par zoom. Contrat versionné `/v1/…`. SQL isolé dans des dépôts de classes, portable vers PostgreSQL/PostGIS.
4. **Tuiles** : fichiers PMTiles statiques (France z15, Europe bas zoom), servis par Apache ; voir `docs/tuiles.md`. Polices et sprites à auto-héberger.
5. **Brancher RuneRide** : `FOND_DE_CARTE` et les routes de ravitaillement de la baseline pointent vers cette API.
6. **Plus tard, hors mutualisé (AWS)** : adresses complètes, itinéraires.

## À confirmer

- Version de MariaDB réelle chez o2switch (testé en local : 10.6 et 11.8), `max_allowed_packet`, quotas disque/IO.
- Limites des CGV pour des fichiers de plusieurs Go (interdit au-delà de 50 Go par fichier) et pour un service public.
- Aucun accès tiers à l'API sans accord d'o2switch.

## État d'avancement (lots 1 et 3)

Fait : `web/` (composer, `Config` `.env` préfixe `CARTO_`, `Database`, `Router`, `public/index.php`, `.htaccess`), routes `/v1/sante`, `/v1/stations`, `/v1/lieux`, jeton facultatif `CARTO_API_KEY`, cache HTTP court, tests PHPUnit (unitaires + intégration sur `runeride_osm`). Les fonctions spatiales MariaDB sont isolées dans `src/Repository/Spatial.php` avec l'équivalent PostGIS en commentaire.

Reste à faire, honnêtement :

- **`min_zoom` et filtrage par zoom** : pas de colonne `min_zoom` dans `osm_lieux`/`osm_stations` ni de paramètre `zoom` dans l'API ; ni écrémage par grille, ni comparaison index spatial / clé de cellule (lot 2).
- Aucun test sur o2switch (version de MariaDB, `ST_Distance_Sphere`, `REF_SYSTEM_ID`, `.htaccess` réel, en-tête `X-Api-Key` transmis par FastCGI). Le `.htaccess` n'a pas été exercé sous Apache, seulement le routage PHP via `php -S`.
- Le portage PostgreSQL/PostGIS est préparé (SQL commun, classe `Spatial`) mais pas essayé.
- Boîte englobante approximative : pas de gestion de l'antiméridien ; recherche de lieux sur `nom` seulement (pas `nom_alt`) ; pas de CORS ni de limitation de débit.
- `lieux` avec préfixe très court (2 lettres) matérialise des dizaines de milliers de lignes : mesuré environ 20 ms en local, à surveiller sur l'hébergeur.
