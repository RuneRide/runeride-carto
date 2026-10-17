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
| `zoom` | facultatif, entier 0..22 : ne renvoie que les stations dont `min_zoom <= zoom` |

Triées par distance croissante (mètres). Exemple : `/v1/stations?lat=43.6045&lon=1.4442&rayon=10&limite=1`

```json
{"donnees":[{"id":"n/4225982857","nom":"station GNVert","marque":null,"exploitant":"GNVert","carburants":["cng"],"adresse":{},"lat":43.6119946,"lon":1.4266636,"min_zoom":11,"distance_m":1640}],
 "meta":{"nombre":1,"limite":1},"attribution":"© les contributeurs d'OpenStreetMap (ODbL) ..."}
```

## GET /v1/lieux

| Paramètre | Règle |
|---|---|
| `q` | requis, préfixe du nom, 2 à 64 caractères, casse et accents ignorés ; `%` et `_` sont littéraux |
| `limite` | défaut 20, 1..100 |
| `lat`, `lon` | facultatifs mais ensemble : ajoute `distance_m` et départage à population égale |
| `zoom` | facultatif, entier 0..22 : ne renvoie que les lieux dont `min_zoom <= zoom` |

Tri : population décroissante (absente = 0), puis proximité. Exemple : `/v1/lieux?q=Toulou&limite=1&lat=43.6&lon=1.44`

```json
{"donnees":[{"id":"n/26686518","nom":"Toulouse","nom_alt":null,"categorie":"city","population":504078,"lat":43.6044638,"lon":1.4442433,"min_zoom":5,"distance_m":603}],
 "meta":{"nombre":1,"limite":1},"attribution":"..."}
```

## GET /v1/vue

Points visibles dans une boîte à un zoom donné, pour alimenter une carte : stations et lieux, `min_zoom <= zoom` (voir `osm.md` pour les règles de `min_zoom` et l'écrémage).

| Paramètre | Règle |
|---|---|
| `zoom` | requis, entier 0..22 |
| `bbox` | requis, `ouest,sud,est,nord` en degrés décimaux (lon -180..180, lat -90..90). `ouest < est` et `sud < nord` (pas d'antiméridien : découper côté client). Taille bornée : au plus 30 x 20 degrés **et** au plus 8 tuiles de 256 px de côté au zoom demandé (`8 x 360 / 2^zoom` degrés : 22,5° à z4, 2,8° à z10, 0,088° à z15) ; sinon `400` |
| `limite` | limite **totale** stations + lieux, défaut 200, 1..500 |

Les plus importants d'abord (`min_zoom` croissant, puis population décroissante) ; si plus de `limite` points sont visibles, les moins importants sont écartés et `meta.tronque` vaut `true`. Pas de `distance_m`. Les stations n'apparaissent qu'à partir de z10 (leur `min_zoom` minimal). Exemple : `/v1/vue?zoom=10&bbox=1.2,43.5,1.7,43.75&limite=3`

```json
{"donnees":{"stations":[],"lieux":[{"id":"n/26686518","nom":"Toulouse","nom_alt":null,"categorie":"city","population":504078,"lat":43.6044638,"lon":1.4442433,"min_zoom":5},
 {"id":"n/26692194","nom":"Colomiers","nom_alt":null,"categorie":"town","population":40159,"lat":43.6112476,"lon":1.3367443,"min_zoom":8}]},
 "meta":{"nombre":3,"limite":3,"tronque":true,"zoom":10,"bbox":[1.2,43.5,1.7,43.75]},"attribution":"..."}
```

(exemple abrégé : la réponse réelle contient aussi Ramonville-Saint-Agne.) Chaque station et chaque lieu porte désormais `min_zoom`. Les paramètres numériques sont validés strictement (`^\d{1,2}$`, pas d'exposant, d'espace ni de retour à la ligne) et tout le SQL passe par des paramètres `?`.

## Lancer et tester

```
cd web && composer install
cp .env.example .env   # renseigner CARTO_DB_*
composer serve         # http://127.0.0.1:8081
CARTO_TEST_DSN='mysql:host=127.0.0.1;port=3307;dbname=runeride_osm;charset=utf8mb4' CARTO_TEST_USER=carto_ro CARTO_TEST_PASSWORD=carto_ro vendor/bin/phpunit
```

Avec le test d'import de bout en bout (base jetable `carto_test_import`, compte administrateur du MariaDB **de test**, `python3` et `mariadb` dans le PATH) :

```
cd web && CARTO_TEST_DSN='mysql:host=127.0.0.1;port=3307;dbname=runeride_osm;charset=utf8mb4' CARTO_TEST_USER=carto_ro CARTO_TEST_PASSWORD=carto_ro \
  CARTO_TEST_ADMIN_HOST=127.0.0.1 CARTO_TEST_ADMIN_PORT=3307 CARTO_TEST_ADMIN_USER=root CARTO_TEST_ADMIN_PASSWORD=root vendor/bin/phpunit
```

Sans `CARTO_TEST_DSN`, les tests d'intégration sont ignorés ; sans `CARTO_TEST_ADMIN_HOST`, seul le test d'import est ignoré. Tests Python (règles, écrémage, échappement) : `python3 -m unittest discover -s bin -p 'test_*.py'`.
