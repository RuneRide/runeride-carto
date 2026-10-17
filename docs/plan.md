# Plan RuneRide Carto

Dépôt séparé de `runeride` (baseline : comptes, trajets). Les outils OSM et le PoC de tuiles en viennent.

## Lots

1. **Socle PHP** (fait, `web/`) : même structure que `runeride/web` (Config, Database, Router, migrations par pilote), base MariaDB distincte de celle des comptes.
2. **Import OSM** : `bin/osm-extraire` (fait, voir `docs/osm.md`) : TSV échappés + `LOAD DATA LOCAL INFILE`, `min_zoom` par point avec écrémage par grille, comparaison mesurée index spatial / index `min_zoom` / clé de cellule (clé de cellule écartée).
3. **API de données** (interne, faite ; voir `docs/api.md`) : stations autour d'un point (avec marque), recherche de lieux par préfixe, paramètre `zoom`, route `/v1/vue` (points visibles dans une boîte). Contrat versionné `/v1/…`. SQL isolé dans des dépôts de classes, portable vers PostgreSQL/PostGIS.
4. **Tuiles** : fichiers PMTiles statiques (France z15, Europe bas zoom), servis par Apache ; voir `docs/tuiles.md`. Polices et sprites à auto-héberger.
5. **Brancher RuneRide** : `FOND_DE_CARTE` et les routes de ravitaillement de la baseline pointent vers cette API.
6. **Plus tard, hors mutualisé (AWS)** : adresses complètes, itinéraires.

## À confirmer

- Version de MariaDB réelle chez o2switch (testé en local : 10.6 et 11.8), `max_allowed_packet`, quotas disque/IO.
- Limites des CGV pour des fichiers de plusieurs Go (interdit au-delà de 50 Go par fichier) et pour un service public.
- Aucun accès tiers à l'API sans accord d'o2switch.

## État d'avancement (lots 1 à 3)

Fait : `web/` (composer, `Config` `.env` préfixe `CARTO_`, `Database`, `Router`, `public/index.php`, `.htaccess`), routes `/v1/sante`, `/v1/stations`, `/v1/lieux`, `/v1/vue`, jeton facultatif `CARTO_API_KEY`, cache HTTP court. Les fonctions spatiales et les indications d'index MariaDB sont isolées dans `src/Repository/Spatial.php` avec l'équivalent PostGIS en commentaire.

Ajouté au lot 2 (17/10/2026) :

- **Import sûr** : plus de `INSERT` construits à la main ; TSV échappés et `LOAD DATA LOCAL INFILE`. Testé de bout en bout avec des noms piégés (apostrophes, antislash, injection SQL, NUL, emoji, retours à la ligne, très longs) dans une base jetable.
- **`min_zoom`** (règles configurables dans `bin/carto_osm.py`, écrémage par grille) calculé à l'import, colonne dans `osm_lieux` et `osm_stations`. Base `runeride_osm` rechargée avec le nouveau schéma (`docs/osm/osm_schema.sql`).
- **Comparaison mesurée** (`bin/bench-zoom.php`, détails et limites dans `docs/osm.md`) : l'index spatial seul lit toute la France à z4 (170 à 245 ms) ; un index sur `min_zoom` règle les zooms bas (0,035 ms à z4) mais pas les hauts (205 ms à z15) ; la clé de cellule n'apporte rien de décisif et est écartée. Stratégie retenue : index `min_zoom` jusqu'à z9, index spatial dès z10.
- **API** : paramètre `zoom` sur `/v1/stations` et `/v1/lieux`, route `/v1/vue` (limite totale 500, boîte bornée en degrés et en tuiles), tests de validation et d'injection.

Reste à faire, honnêtement :

- **Qualité cartographique des règles de `min_zoom`** : seuils choisis a priori, jamais confrontés à un rendu de carte ; stations absentes avant z10 par construction.
- Aucun test sur o2switch (version de MariaDB, `local_infile` pour le chargement, `ST_Distance_Sphere`, `REF_SYSTEM_ID`, `FORCE INDEX`, `.htaccess` réel, en-tête `X-Api-Key` transmis par FastCGI). Le `.htaccess` n'a pas été exercé sous Apache, seulement le routage PHP via `php -S`.
- Mesures de comparaison : caches chauds, une seule machine, base seule (sans le tri/`LIMIT` de l'API), pas de charge concurrente ni de cache froid.
- Le portage PostgreSQL/PostGIS est préparé (SQL commun, classe `Spatial`) mais pas essayé.
- Boîte englobante approximative : pas de gestion de l'antiméridien (`/v1/vue` le refuse) ; recherche de lieux sur `nom` seulement (pas `nom_alt`) ; pas de CORS ni de limitation de débit.
- `lieux` avec préfixe très court (2 lettres) matérialise des dizaines de milliers de lignes : mesuré environ 20 ms en local (première version), à surveiller sur l'hébergeur.
