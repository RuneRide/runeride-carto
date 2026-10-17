-- Schéma des tables OSM de RuneRide (MariaDB >= 10.5, InnoDB, utf8mb4).
-- Données © contributeurs OpenStreetMap, licence ODbL.
-- Volontairement hors de web/migrations/mariadb/ : ces tables sont un jeu de
-- données de référence, rechargé en bloc (TRUNCATE + dump), pas un état
-- applicatif versionné ; elles n'ont rien à faire dans la numérotation de
-- bin/migrer et ne doivent pas être appliquées automatiquement.
-- min_zoom : zoom minimal d'affichage du point (règles et écrémage : bin/carto_osm.py).
-- Index min_zoom : utile aux zooms bas (peu de lignes) ; l'index spatial sert aux zooms hauts (mesures : docs/osm.md).
-- Pour une base existante créée avant min_zoom : DROP TABLE puis recréer et recharger (le jeu est reconstruit en bloc).
-- Portage PostGIS : DOUBLE -> double precision, POINT ... -> geography(Point,4326),
-- SPATIAL INDEX -> GIST, collation -> citext/unaccent + text_pattern_ops.

CREATE TABLE IF NOT EXISTS osm_stations (
  osm_type   CHAR(1)      NOT NULL COMMENT 'n = noeud, w = chemin',
  osm_id     BIGINT       NOT NULL,
  nom        VARCHAR(255) NULL,
  marque     VARCHAR(255) NULL,
  exploitant VARCHAR(255) NULL,
  carburants VARCHAR(255) NULL COMMENT 'clés fuel:* = yes, séparées par des virgules',
  adr_numero VARCHAR(32)  NULL,
  adr_rue    VARCHAR(255) NULL,
  adr_cp     VARCHAR(32)  NULL,
  adr_ville  VARCHAR(255) NULL,
  lat        DOUBLE       NOT NULL,
  lon        DOUBLE       NOT NULL,
  geom       POINT        REF_SYSTEM_ID=4326 NOT NULL COMMENT 'POINT(lon lat)',
  min_zoom   TINYINT UNSIGNED NOT NULL DEFAULT 15 COMMENT 'zoom minimal d affichage (règles : bin/carto_osm.py)',
  PRIMARY KEY (osm_type, osm_id),
  SPATIAL INDEX sx_osm_stations_geom (geom),
  INDEX ix_osm_stations_min_zoom (min_zoom),
  INDEX ix_osm_stations_marque (marque(64)),
  INDEX ix_osm_stations_nom (nom(64))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS osm_lieux (
  osm_type   CHAR(1)      NOT NULL,
  osm_id     BIGINT       NOT NULL,
  categorie  VARCHAR(16)  NOT NULL COMMENT 'place=city|town|village|hamlet|suburb|quarter|neighbourhood',
  nom        VARCHAR(255) NOT NULL,
  nom_alt    VARCHAR(512) NULL,
  population INT          NULL,
  lat        DOUBLE       NOT NULL,
  lon        DOUBLE       NOT NULL,
  geom       POINT        REF_SYSTEM_ID=4326 NOT NULL COMMENT 'POINT(lon lat)',
  min_zoom   TINYINT UNSIGNED NOT NULL DEFAULT 15 COMMENT 'zoom minimal d affichage (règles : bin/carto_osm.py)',
  PRIMARY KEY (osm_type, osm_id),
  SPATIAL INDEX sx_osm_lieux_geom (geom),
  INDEX ix_osm_lieux_min_zoom (min_zoom),
  INDEX ix_osm_lieux_nom (nom(64)),
  INDEX ix_osm_lieux_categorie (categorie)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
