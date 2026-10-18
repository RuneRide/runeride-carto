// SPDX-License-Identifier: EUPL-1.2
//! Fonction Lambda Raido (spec `runeride/docs/itineraire.md` §5.2, §7.2) : un binaire `bootstrap`
//! derrière une Function URL (elle-même derrière CloudFront avec OAC). `s3` lit les tuiles de
//! graphe sur S3 à travers le cache `/tmp` ; `handler` traduit l'événement HTTP en requête Raido.

pub mod handler;
pub mod s3;
