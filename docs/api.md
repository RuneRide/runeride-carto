# API Carto v1 (interne)

API **interne à RuneRide**, en lecture seule, JSON UTF-8. Non destinée à des tiers (voir les limites des CGV d'o2switch dans `plan.md`). Si `CARTO_API_KEY` est défini côté serveur, chaque requête doit envoyer `X-Api-Key: <jeton>` (sinon `401`) ; ce jeton partagé n'est pas une authentification d'utilisateurs. Toute réponse de données porte l'attribution ODbL (`attribution`), à afficher par l'application.

Cache : `Cache-Control: public, max-age=300` (réglable par `CARTO_CACHE_SECONDS`) sur les `200` de données ; `no-store` pour `/v1/sante` et les erreurs.

Erreurs : `400` avec `{"erreur": "...", "parametre": "lat"}` pour un paramètre invalide, `401` jeton, `404` route inconnue, `405` méthode, `503` base indisponible (sans détail).

## GET /v1/sante

`{"statut":"ok","env":"local","version":"v1"}` (interroge la base : `503` si elle est injoignable).

## GET /v1/stations

| Paramètre | Règle |
|---|---|
| `lat`, `lon` | requis, décimaux, -90..90 et -180..180 |
| `rayon` | km, défaut 10, > 0 et <= 50 |
| `marque` | facultatif, égalité (casse et accents ignorés), 64 car. au plus |
| `limite` | défaut 50, 1..100 |

Triées par distance croissante (mètres). Exemple : `/v1/stations?lat=43.6045&lon=1.4442&rayon=10&limite=1`

```json
{"donnees":[{"id":"n/4225982857","nom":"station GNVert","marque":null,"exploitant":"GNVert","carburants":["cng"],"adresse":{},"lat":43.6119946,"lon":1.4266636,"distance_m":1640}],
 "meta":{"nombre":1,"limite":1},"attribution":"© les contributeurs d'OpenStreetMap (ODbL) ..."}
```

## GET /v1/lieux

| Paramètre | Règle |
|---|---|
| `q` | requis, préfixe du nom, 2 à 64 caractères, casse et accents ignorés ; `%` et `_` sont littéraux |
| `limite` | défaut 20, 1..100 |
| `lat`, `lon` | facultatifs mais ensemble : ajoute `distance_m` et départage à population égale |

Tri : population décroissante (absente = 0), puis proximité. Exemple : `/v1/lieux?q=Toulou&limite=1&lat=43.6&lon=1.44`

```json
{"donnees":[{"id":"n/26686518","nom":"Toulouse","nom_alt":null,"categorie":"city","population":504078,"lat":43.6044638,"lon":1.4442433,"distance_m":603}],
 "meta":{"nombre":1,"limite":1},"attribution":"..."}
```

## Lancer et tester

```
cd web && composer install
cp .env.example .env   # renseigner CARTO_DB_*
composer serve         # http://127.0.0.1:8081
CARTO_TEST_DSN='mysql:host=127.0.0.1;port=3307;dbname=runeride_osm;charset=utf8mb4' CARTO_TEST_USER=carto_ro CARTO_TEST_PASSWORD=carto_ro vendor/bin/phpunit
```

Sans `CARTO_TEST_DSN`, les tests d'intégration sont ignorés.
