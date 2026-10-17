# RuneRide Carto

Données cartographiques de RuneRide, prévues pour `carto.runeride.fr` : fond de carte (tuiles PMTiles statiques) et API de données OSM (stations-service avec marque, lieux et localités).

- Pré-traitement **en local**, jamais sur l'hébergeur : `bin/osm-extraire` filtre un extrait Geofabrik et produit des fichiers TSV et un `charger.sql` (voir `docs/osm.md`).
- Hébergement visé : o2switch (PHP 8 + MariaDB, fichiers statiques). Les parties lourdes (adresses complètes, itinéraires) iraient sur AWS plus tard, derrière le même nom.
- API **interne** à RuneRide pour l'instant : ouvrir à des tiers demanderait de revoir les CGV d'o2switch.
- Données © contributeurs OpenStreetMap, licence ODbL (attribution obligatoire dans toute application qui les affiche).

Plan : [docs/plan.md](docs/plan.md)
