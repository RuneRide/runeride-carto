// SPDX-License-Identifier: EUPL-1.2
//! Pipeline de construction du graphe Raido : `osm` lit le PBF, `scores` note les
//! tronçons, `construire` découpe aux intersections et écrit les tuiles.
//! Traitement 100 % local, comme `bin/osm-extraire`.  Données © contributeurs
//! OpenStreetMap (ODbL).

pub mod construire;
pub mod osm;
pub mod scores;
