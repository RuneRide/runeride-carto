// SPDX-License-Identifier: EUPL-1.2
//! Point d'entrée de la fonction Lambda Raido (runtime `provided.al2023`, exécutable nommé `bootstrap`).
//!
//! Mode de test sans runtime Lambda :
//!   `bootstrap --requete requete.json [--chemin /v1/balade]`   → traite la requête et écrit la réponse HTTP
//!   `bootstrap --evenement evenement.json`                      → rejoue un événement Function URL complet
//! avec la configuration des variables d'environnement (`RAIDO_SOURCE=local:/chemin/du/graphe` en local).

use std::sync::Arc;

use lambda_runtime::{run, service_fn, Error, LambdaEvent};
use raido_lambda::handler::{journal, traiter_http, Contexte};
use serde_json::{json, Value};

async fn gestionnaire(ctx: Arc<Contexte>, evt: LambdaEvent<Value>) -> Result<Value, Error> {
    // calcul CPU et lectures S3 synchrones : hors des fils du runtime asynchrone
    Ok(tokio::task::spawn_blocking(move || traiter_http(&ctx, &evt.payload)).await?)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    let ctx = match Contexte::depuis_env() {
        Ok(c) => Arc::new(c),
        Err(e) => {
            journal("ERROR", json!({"evenement": "configuration", "erreur": format!("{e:#}")}));
            return Err(e.into());
        }
    };
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--requete") => {
            let chemin = args.get(2).ok_or("--requete attend un fichier")?;
            let route = args.iter().position(|a| a == "--chemin").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "/v1/balade".into());
            let corps = std::fs::read_to_string(chemin)?;
            let evt = json!({"rawPath": route, "requestContext": {"http": {"method": "POST"}}, "body": corps, "isBase64Encoded": false});
            let rep = traiter_http(&ctx, &evt);
            println!("{}", serde_json::to_string_pretty(&rep)?);
            std::process::exit(if rep.get("statusCode").and_then(Value::as_u64) == Some(200) { 0 } else { 3 });
        }
        Some("--evenement") => {
            let chemin = args.get(2).ok_or("--evenement attend un fichier")?;
            let evt: Value = serde_json::from_slice(&std::fs::read(chemin)?)?;
            println!("{}", serde_json::to_string_pretty(&traiter_http(&ctx, &evt))?);
            return Ok(());
        }
        _ => {}
    }
    run(service_fn(move |evt| gestionnaire(ctx.clone(), evt))).await
}
