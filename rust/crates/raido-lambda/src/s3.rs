// SPDX-License-Identifier: EUPL-1.2
//! Source de tuiles S3 : `GetObject` signé SigV4 par `rw-aws`, identifiants du rôle d'exécution.

use anyhow::{anyhow, Result};
use raido_calcul::charge::SourceTuiles;
use rw_aws::s3::{ConfigS3, Identifiants, StockageS3};
use rw_aws::stockage::Stockage;

pub struct SourceS3 {
    stockage: StockageS3,
    prefixe: String,
    bucket: String,
}

impl SourceS3 {
    /// `prefixe` : dossier du graphe dans le bucket (par exemple `graphe/courant`), sans barre finale.
    pub fn nouvelle(bucket: &str, region: &str, prefixe: &str, endpoint: Option<String>) -> Result<SourceS3> {
        let mut cfg = ConfigS3::aws(bucket, region);
        cfg.endpoint = endpoint;
        let stockage = StockageS3::new(cfg, Identifiants::depuis_env().map_err(|e| anyhow!("{e}"))?).map_err(|e| anyhow!("{e}"))?;
        Ok(SourceS3 { stockage, prefixe: prefixe.trim_matches('/').to_string(), bucket: bucket.to_string() })
    }
}

impl SourceTuiles for SourceS3 {
    fn lire(&self, chemin: &str) -> Result<Option<Vec<u8>>> {
        let cle = if self.prefixe.is_empty() { chemin.to_string() } else { format!("{}/{}", self.prefixe, chemin) };
        self.stockage.lire(&cle).map_err(|e| anyhow!("S3 {cle} : {e}"))
    }
    fn nom(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.prefixe)
    }
}
