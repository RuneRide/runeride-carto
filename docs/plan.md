# Plan RuneRide Carto

Dépôt séparé de `runeride` (baseline : comptes, trajets). Les outils OSM et le PoC de tuiles en viennent.

## Lots

1. **Socle PHP** (fait, `web/`) : même structure que `runeride/web` (Config, Database, Router, migrations par pilote), base MariaDB distincte de celle des comptes.
2. **Import OSM** : `bin/osm-extraire` (fait, voir `docs/osm.md`) : TSV échappés + `LOAD DATA LOCAL INFILE`, `min_zoom` par point avec écrémage par grille, comparaison mesurée index spatial / index `min_zoom` / clé de cellule (clé de cellule écartée).
3. **API de données** (interne, faite ; voir `docs/api.md`) : stations autour d'un point (avec marque), recherche de lieux par préfixe, paramètre `zoom`, route `/v1/vue` (points visibles dans une boîte). Contrat versionné `/v1/…`. SQL isolé dans des dépôts de classes (Doctrine DBAL), portable vers PostgreSQL/PostGIS.
4. **Tuiles** : fichiers PMTiles statiques (France z15, Europe bas zoom), servis par Apache ; voir `docs/tuiles.md`. Polices et sprites à auto-héberger.
5. **Brancher RuneRide** : `FOND_DE_CARTE` et les routes de ravitaillement de la baseline pointent vers cette API.
6. **Raido, calculateur de balade** (spécification dans `runeride/docs/itineraire.md` ; PoC J1 fait le 18/10/2026, voir `docs/raido.md`) : pipeline `rust/crates/raido-graphe` (PBF → tuiles de graphe rkyv z8, scores sinuosité et agglomération, restrictions, composantes), moteur `rust/crates/raido-calcul` (A\* par arcs, boucles par pétales, banc de mesure). Mesuré sur Rhône-Alpes : graphe en 6 s (25 tuiles z9, max 14,5 Mo), boucle de 120 km en 1,8 s médiane et 2,9 s au p95 sur un fil. J3 écrit le même jour (caisse `raido-lambda` : Function URL, S3 signé via `rw-aws`, cache `/tmp`, zip arm64 de 1,3 Mo ; module Terraform `runeride-cloud/terraform/raido/` validé hors ligne), rien de déployé. Reste : vérification du jeton court, déploiement d'essai, scores scénique et motard, calibration des vitesses sur GeoRide, étapes, météo, découverte. Hors mutualisé (AWS), comme les adresses complètes.

- **CORS** : fait (`Cors.php`, `CARTO_CORS_ORIGINS`, tests) ; le serveur de production doit encore recevoir la variable et les fichiers.

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

## Accès SQL : Doctrine DBAL (17/10/2026)

Adopté : `doctrine/dbal` 4.5.0 (bibliothèque seule, pas l'ORM ; exige PHP >= 8.2, compatible avec le `>=8.3` du projet). `Database::connect()` renvoie une `Doctrine\DBAL\Connection` paresseuse construite depuis le même DSN PDO du `.env` (pilotes `mysql` et `pgsql`). `StationRepository` et `LieuRepository` utilisent le `QueryBuilder` : valeurs toujours en paramètres nommés (`setParameter`), `LIMIT` via `setMaxResults`, plus aucune concaténation de valeur. Les fonctions spatiales MariaDB et les `FORCE INDEX` restent dans `Spatial.php` (fragments constants à paramètres nommés `:point`, `:boite`, `:zoom`...), avec l'équivalent PostGIS en commentaire ; stratégie par zoom inchangée (index `min_zoom` jusqu'à z9, index spatial dès z10).

Vérifié : 72 tests PHPUnit verts sans changer une assertion (seul l'amorçage des deux tests d'intégration change : connexion DBAL, PDO natif via `getNativeConnection()` pour leurs vérifications directes) ; `EXPLAIN` identique avant/après sur 4 requêtes représentatives (stations autour, vue z5 stations, vue z12 lieux, préfixe de lieux) et résultats identiques (même empreinte) ; charges d'injection rejouées contre `php -S` (apostrophes, `DROP TABLE`, `UNION`, `%%`, `limite`/`zoom`/`lat`/`rayon`/`bbox` piégés) : rejets 400 ou listes vides, tables intactes.

Reste à faire :

- **Baseline** (dépôt `runeride`, non touché ici) : y adopter la même approche DBAL, puis aligner `FOND_DE_CARTE` et les routes de ravitaillement sur cette API (lot 5).
- Envoyer `vendor/` (avec `doctrine/dbal`, `psr/log`, `psr/cache`, `doctrine/deprecations`) sur l'hébergeur ; vérifier `pdo_mysql` activé chez o2switch.
- Le portage PostgreSQL/PostGIS reste à essayer : seuls `Spatial.php` et les `FORCE INDEX` sont à réécrire, mais aucun test sur PostgreSQL n'a été fait.
- `bin/bench-zoom.php` reste en PDO brut (hors API, non migré).
