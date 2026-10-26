// SPDX-License-Identifier: EUPL-1.2
//! Moteur de calcul Raido (spec `runeride/docs/itineraire.md` §5.4).
//!
//! `charge` lit les tuiles rkyv à la demande, `cout` traduit les préférences en
//! fonction de coût, `astar` cherche un chemin par arcs (restrictions de tourner),
//! `boucle` fabrique des balades par pétales.  Aucune dépendance au réseau ici :
//! le chargement S3 viendra avec la Lambda (jalon J3).

pub mod apparier;
pub mod astar;
pub mod boucle;
pub mod charge;
pub mod cout;
pub mod geo;
pub mod meteo;
pub mod polyligne;
pub mod requete;
