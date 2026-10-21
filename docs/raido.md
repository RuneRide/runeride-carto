<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Raido : graphe routier et moteur de balade (PoC J1)

Statut : **jalon J1 de [runeride/docs/itineraire.md](../../runeride/docs/itineraire.md) réalisé et accepté le 18/10/2026** (tracés validés par le titulaire), tout en local, rien n'est déployé. Ce document décrit ce qui existe dans `rust/`, les mesures obtenues sur l'extrait Rhône-Alpes et ce qui n'a pas été fait. Vocabulaire : **mesuré** / **ESTIMATION** / **OUVERT**.

## 1. Ce qui existe

Espace de travail Cargo `rust/` (même organisation que `runeride-weather/rust`), quatre caisses sous EUPL-1.2 :

| Caisse | Rôle | Fichiers |
|---|---|---|
| `raido-format` | types des tuiles de graphe (`rkyv` 0.8), identifiants globaux, découpage z8, haversine, géohash | `crates/raido-format/src/lib.rs` |
| `raido-graphe` | pipeline : lecture du PBF (`osmpbf`, deux passes parallèles), découpage aux intersections, scores, restrictions de tourner, composantes connexes, écriture des tuiles et de `index.json` | `osm.rs`, `scores.rs`, `construire.rs`, `main.rs` |
| `raido-calcul` | moteur : chargement des tuiles à la demande (source disque, S3 ou cache ; validation rkyv au chargement, accès zéro copie ensuite), fonction de coût, A\* par arcs avec restrictions et interdiction de demi-tour, boucles par pétales parallélisées (`rayon`), contrat JSON de l'API, ligne de commande `trajet` / `balade` / `banc` | `charge.rs`, `cout.rs`, `astar.rs`, `boucle.rs`, `geo.rs`, `requete.rs`, `polyligne.rs`, `main.rs` |
| `raido-lambda` | fonction Lambda (§2.5) : source S3 signée, handler Function URL, binaire `bootstrap` | `crates/raido-lambda/`, `tools/package-lambda.sh` |

Usage :

```
cargo build --release
./target/release/raido-graphe construire ../donnees/pbf/rhone-alpes-latest.osm.pbf ../donnees/graphe/rhone-alpes
./target/release/raido-calcul -g ../donnees/graphe/rhone-alpes balade 45.1885,5.7245 --distance-km 120 --geojson balade.geojson
./target/release/raido-calcul -g ../donnees/graphe/rhone-alpes trajet 45.7640,4.8357 45.1885,5.7245 --rapide --autoroutes
./target/release/raido-calcul -g ../donnees/graphe/rhone-alpes banc --n 30 --distance-km 120 --fils 1
```

`donnees/` (PBF et graphes) est ignoré par Git. Données © contributeurs OpenStreetMap (ODbL).

### 1.1 Format des tuiles (version 3)

- Tuile **z9** Web Mercator (≈ 75 × 50 km en France ; z8 en J1, voir §2.4) ; nœuds (intersections) en CSR, arcs sortants, géométrie intermédiaire partagée entre l'arc aller et l'arc retour (drapeau `GEOM_INVERSEE`), restrictions de tourner triées par nœud pivot.
- Identifiants globaux `cle_tuile << 32 | indice_local` pour les nœuds et les arcs ; un arc franchissant une frontière de tuile vit dans la tuile de sa source.
- Par arc : cible, arc inverse, longueur (m), temps de base (ds), classe, drapeaux (péage, tunnel, non revêtu, revêtement inconnu, rond-point), 4 scores sur 8 bits (sinuosité, scénique, motard, agglomération), `way_id` + indice de segment et géohash du milieu (identifiant stable, spec §5.3).
- Par nœud : position en 1e-7°, drapeau `PRINCIPAL` (composante connexe principale).

### 1.2 Règles d'extraction (`osm.rs`)

- Classes gardées : `motorway` … `residential`, `living_street`, bretelles. Exclus : `service`, `track`, `path` et toute voie `access`/`motor_vehicle` = `no`/`private` sauf `motorcycle=yes`.
- Sens unique : `oneway`, implicite sur `motorway`, ronds-points et `motorway_link`.
- `maxspeed` lu (dont `FR:urban`, `FR:rural`, mph) ; `surface` et `smoothness` → `NON_REVETU` ; `surface` absent sur une petite route → `REVETEMENT_INCONNU` (informatif).
- Restrictions `type=restriction` avec pivot nœud (`no_*`, `only_*`), `except=motorcycle` respecté ; pivot « way » ignoré.
- Localités `city`/`town`/`village` avec population (défauts si absente) pour le score d'agglomération.

### 1.3 Scores et coût (`scores.rs`, `cout.rs`)

- **Sinuosité** : radians de changement de cap par kilomètre, saturée à 4 rad/km, plafonnée à 0,5 sous 150 m.
- **Agglomération** : 1 dans un rayon `13,5 × √population` m autour des localités ≥ 5 000 habitants, décroissant à 0 à 2 km de la limite.
- **Scénique**, **motard** : 0 (jalons J2 et J7).
- Temps de base : vitesse par classe (115/90/80/75/65/55/40/60/20 km/h, provisoire), bornée par `maxspeed`, ralentie jusqu'à −25 % par la sinuosité, 30 km/h dans les ronds-points.
- Coût = `temps × (1 + Σ poids × (1 − score) + poids_agglo × agglo [+ malus bretelle/résidentiel] [+ découverte])`, × 4 pour les arcs déjà empruntés par la boucle en cours ; exclusions dures : autoroutes, péages, non revêtu, tunnels.

### 1.4 Algorithmes (`astar.rs`, `boucle.rs`)

- A\* unidirectionnel **par arcs** (état = arc d'arrivée), heuristique distance / 110 km/h en décisecondes (admissible, le coût est ≥ temps), `epsilon` optionnel pour un A\* pondéré.
- Virages : restrictions `no_*`/`only_*` du nœud pivot, demi-tour interdit sauf impasse. Un arc vers une tuile non chargée est infranchissable.
- Boucle : 12 caps × 2 rayons = 24 pétales, triangle `D → P1 → P2 → D` avec rayon `L/3 × 0,72`, recouvrement pénalisé entre branches, note composite (écart de distance, recouvrement, agglomération, bonus sinuosité) ; pétales calculés en parallèle ; les trois meilleurs sont rejoués avec un rayon corrigé si leur longueur s'écarte de plus de 4 % de la cible.
- Nœud d'accroche : le plus proche dans la composante principale, avec un arc double sens franchissable et ≥ 60 nœuds joignables malgré les exclusions ; rayon 3 km puis 9 km, sinon erreur explicite.

## 2. Mesures (18/10/2026, portable Linux 20 cœurs, disque local)

### 2.1 Construction (Rhône-Alpes, `rhone-alpes-latest.osm.pbf` du 18/10/2026)

| Mesure | Valeur |
|---|---|
| Lecture du PBF (deux passes) | 439 k voies carrossables, 8 047 restrictions, 3 016 localités, 4,56 M nœuds, **3,0 s** |
| Construction et écriture | **2,9 s** |
| Mémoire maximale du pipeline | **1,26 Go** |
| Graphe | 543 118 intersections, 1 169 900 arcs, 10 tuiles z8, **116 Mo** |
| Tuiles | de 0,3 à **41 Mo** (Lyon, 131/91) ; 25 Mo (132/91, Savoie), 20 Mo (131/92, Drôme–Ardèche) |
| Restrictions résolues | 7 006 / 8 047 (87 % ; le reste a un pivot « way ») |
| Composantes connexes | 1 073, dont 2 819 nœuds (0,5 %) hors de la principale |

### 2.2 Calcul (release, à chaud sauf mention)

| Cas | Résultat | Temps |
|---|---|---|
| Lyon → Grenoble, le plus rapide (autoroute) | 106 km, 72 min, 104 k états | **42 ms** |
| Lyon → Grenoble, mode balade (sans autoroute ni péage) | 113 km, 123 min, 592 k états | **0,27 s** |
| Lyon Bellecour → Part-Dieu, mode balade | 3,4 km, 4,6 k états | 2 ms |
| Boucle 120 km depuis Grenoble, 1 fil | 22 candidats, meilleur 128 km, recouvrement 1 %, sinuosité 0,63 | 2,6 s |
| Préchargement de 4 à 7 tuiles (73 à 108 Mo) depuis le disque | — | 0,05 à 0,08 s |
| Mémoire maximale du processus pour des balades de 120 km (tuiles comprises) | **152 Mo** | — |
| Taille des binaires release (`strip`, LTO) | 1,0 Mo chacun | — |

Bancs (`banc`, départs tirés au sort dans l'emprise, réglage `Preferences::balade`, **1 fil** = équivalent d'une Lambda de 2 Go) :

| Boucle | Départs | Échecs | Médiane | p95 | Max |
|---|---|---|---|---|---|
| 60 km | 30 | 0 | **0,80 s** | 1,72 s | 1,78 s |
| 120 km (3 graines) | 90 | 0 | **2,1 à 2,3 s** | **3,8 à 4,1 s** | 4,3 s |
| 120 km à froid (cache vidé, disque local) | 10 | 0 | 2,48 s | 3,99 s | 3,99 s (76 Mo lus par boucle) |
| 200 km | 15 | 0 | 3,36 s | 7,07 s | 7,42 s |
| 120 km, 20 fils | 20 | 0 | **0,33 s** | 0,90 s | 1,19 s |
| 120 km, 1 fil, `epsilon` 1,5 | 20 | 0 | 2,11 s | 3,60 s | 6,33 s |

Lecture : la cible « < 4 s à chaud » (spec §8) est **tenue en médiane et au p95 pour 120 km sur un fil**, pas pour 200 km. Le parallélisme des pétales divise le temps par 7 sur 20 cœurs ; dans Lambda, la puissance CPU suit la mémoire (≈ 1 vCPU à 1,8 Go, ≈ 3,5 vCPU à 6 Go), donc **augmenter la mémoire réduit la latence à coût en Go·s égal** : c'est la parade naturelle pour les grandes boucles. L'A\* pondéré n'apporte que 7 % : le coût pondéré domine l'heuristique « temps », il n'y a rien à gagner de ce côté sans points de repère.

### 2.3 Ce que J1 apprend pour la suite

- **Tuiles trop grosses** (z8) : 41 Mo pour Lyon contre 10 Mo visés (spec §5.3). **Réglé le soir même par le passage en z9** (§2.4) : max 14,5 Mo, 54 Mo lus à froid.
- **Durées trop longues ?** 120 km de boucle en montagne donnaient 150 à 200 min, soit 40 à 50 km/h de moyenne. **Vitesses relevées** (§2.4), 52 à 55 km/h désormais ; à calibrer pour de bon sur les trajets GeoRide réels (spec O3).
- **Îlots** : 0,5 % des nœuds sont hors de la composante principale (hameaux reliés par des pistes exclues). Le drapeau `PRINCIPAL` règle l'accroche ; la haute montagne sans route à 9 km renvoie une erreur explicite.
- **Boucle** : 24 pétales suffisent à produire 18 à 24 candidats avec un recouvrement de 0 à 5 % ; la variante retenue dépassait la distance cible de 5 à 15 % avec le facteur 0,8. **Réglé** par le facteur 0,72 et le raffinement des trois meilleurs (§2.4) : +3 à +5 %.

### 2.4 Itération du 18/10/2026 au soir : z9, vitesses, raffinement (format v3)

Trois changements après l'acceptation de J1, mêmes bancs (1 fil, mêmes graines) :

| Changement | Avant | Après |
|---|---|---|
| Tuiles **z9** au lieu de z8 (clé `x << 16 \| y`) | 10 tuiles, max **41 Mo**, 76 Mo lus par boucle à froid | **25 tuiles, médiane 4,1 Mo, max 14,5 Mo** (Lyon et Grenoble restent > 10 Mo), **54 Mo** lus à froid |
| Vitesses par classe relevées (115/90/80/75/65/55/40/60/20 km/h), `maxspeed` pris tel quel, sinuosité −25 % au plus, ronds-points 30 km/h | 43 km/h de moyenne sur les boucles | **52 à 55 km/h** |
| Pétales : rayon 0,72 × L/3 (au lieu de 0,8) + **raffinement** des 3 meilleurs candidats (rayon corrigé du rapport visé / obtenu quand l'écart dépasse 4 %) | boucles de +5 à +15 % trop longues | **+3 à +5 % en moyenne, \|écart\| 6 %** |

| Boucle, 1 fil | Médiane | p95 | Max | Écart moyen |
|---|---|---|---|---|
| 60 km (30) | **0,53 s** | 0,97 s | 1,44 s | +4,3 % |
| 120 km (30) | **1,79 s** | **2,90 s** | 3,03 s | +4,5 % |
| 120 km à froid (10) | 1,87 s | 3,16 s | 3,16 s | +5,4 % |
| 200 km (15) | 3,75 s | 6,31 s | 6,52 s | +3,1 % |
| 120 km, 20 fils (30) | 0,29 s | 0,47 s | 0,49 s | +4,5 % |

Le gain de temps à 120 km (p95 de 4,0 à 2,9 s) vient surtout des tuiles plus petites (moins d'octets touchés, meilleure localité) ; le raffinement coûte ≈ 0,1 s. Les vitesses restent des valeurs a priori : seule la calibration GeoRide (spec O3) les rendra justes.

## 2.5 Jalon J3 (18/10/2026 au soir) : fonction Lambda, S3, Terraform

Rien n'est déployé : tout ce qui suit a été compilé et exercé **en local**, dont la source S3 contre un faux S3 qui vérifie la signature.

### Ce qui existe

| Élément | Fichier |
|---|---|
| Source de tuiles abstraite : disque, **cache `/tmp` devant une autre source** (fichier temporaire + `rename`) | `raido-calcul/src/charge.rs` (`SourceTuiles`, `SourceDisque`, `SourceCache`) |
| Contrat JSON de `POST /v1/balade` et `/v1/trajet` (spec §7.2, sous-ensemble J3 : départ, retour ou arrivée, distance ou durée, préférences, évitements, tronçons connus, variante) ; réponse avec polyligne encodée, scores, avertissements, mesures ; erreurs 400 / 422 / 500 typées | `raido-calcul/src/requete.rs`, `polyligne.rs` |
| Caisse **`raido-lambda`** : source S3 (`GetObject` SigV4 via `rw-aws` de runeride-weather, dépendance `git` épinglée sur `f981a47`), handler Function URL (format API Gateway v2, routes `GET /v1/sante`, `POST /v1/balade`, `POST /v1/trajet`, `OPTIONS`), journal JSON sur la sortie d'erreur, binaire `bootstrap` (`lambda_runtime`, Tokio `current_thread`, calcul en `spawn_blocking`) avec modes locaux `--requete` et `--evenement` | `crates/raido-lambda/` |
| Emballage arm64 musl sans compilateur C (TLS en Rust pur, `rust-lld`) | `rust/tools/package-lambda.sh` |
| Module Terraform `raido/` : S3 privé, Lambda arm64 + Function URL `AWS_IAM`, CloudFront avec **OAC de type `lambda`**, CORS, alarmes, budget, README | `runeride-cloud/terraform/raido/` |

Variables d'environnement de la fonction : `RAIDO_SOURCE` (`s3` ou `local:/chemin`), `RAIDO_BUCKET`, `RAIDO_PREFIXE` (`graphe/courant`), `RAIDO_CACHE` (`/tmp/raido`), `RAIDO_S3_ENDPOINT` (tests), `RAIDO_THREADS`, `AWS_REGION` et les identifiants fournis par Lambda ; jeton : `RAIDO_JETON` (`obligatoire` | `desactive`), `RAIDO_JWKS_URL` ou `RAIDO_JWKS_FICHIER`, `RAIDO_EMETTEUR`, `RAIDO_AUDIENCE`.

**Jeton court** (`raido-lambda/src/jeton.rs`, spec §7.1) : JWT RS256 au format exact de la classe PHP `RuneRide\Auth\Jwt` (en-tête `kid` = 16 premiers hexadécimaux de sha256 du PEM public, JWKS `/.well-known/jwks.json`), vérifié avec la caisse `rsa` 0.9 (Rust pur). Revendications : `iss`, `aud` = `raido` (les jetons de session `aud = runeride` sont refusés), `sub` (jamais journalisé), `iat`, `exp` (tolérance 60 s), `palier` (`gratuit` | `abonne`), `type` (`petite_boucle` | `grand`), `limite_km` (défaut 25). Application : `petite_boucle` → `/v1/balade` seul et distance cible ≤ `limite_km`, sinon `403 quota_requis` ; `palier` ≠ `abonne` et découverte demandée → `403 abonnement_requis` ; jeton absent → `401 jeton_requis` ; invalide → `401 jeton_invalide`. Le JWKS est lu au démarrage et rechargé sur `kid` inconnu (au plus toutes les 5 min). La **délivrance** (`POST /api/balade/jeton`, PHP) est faite depuis le 19/10/2026 : §2.6.

### Mesures (locales)

| Mesure | Valeur |
|---|---|
| Binaire `bootstrap` arm64 (`aarch64-unknown-linux-musl`, LTO, strippé) | **2,5 Mo** ; zip **1,3 Mo** |
| Balade 180 min depuis Grenoble, source disque, 1 fil | 135 km, 3 h 11 avec pauses, 3,0 s de calcul, 15 tuiles (90 Mo) |
| Idem, **source S3 (faux S3 local, SigV4 vérifiée)**, cache `/tmp` vide | chargement **204 ms** (16 `GET` signés), 3,25 s au total, 148 Mo de RSS |
| Idem, cache `/tmp` chaud | chargement **54 ms**, aucune requête S3 |
| Mauvaise signature | `403 SignatureDoesNotMatch` remonté dans le journal, réponse 500 propre |
| Lyon → Grenoble (`/v1/trajet`) | 111 km, 0,14 s |
| `terraform fmt` / `init -backend=false` / `validate` | succès, fournisseur aws 6.69.1 |

### Ce qui n'est pas fait

- **Aucun appel à AWS** : ni `plan`, ni `apply`, ni S3 réel. Le TLS en Rust pur face aux points de terminaison d'AWS n'est pas vérifié (test jetable de runeride-weather en attente).
- ~~Délivrance du jeton court côté PHP~~ : faite le 19/10/2026 (§2.6), chaîne PHP → Lambda exercée en local avec la vraie classe `Jwt`.
- Étapes, autonomie, météo, découverte par `troncons_url`, GPX, `/v1/lieu`, `/v1/apparier` : jalons suivants.
- Le `POST` derrière un OAC Lambda exige l'en-tête `x-amz-content-sha256` côté client (README du module) : non exercé.

## 2.6 Jalon J4, côté PHP (19/10/2026) : délivrance du jeton, quotas, balades enregistrées

Fait dans `runeride` (PR #9 et #11, fusionnées sur `main`), spec §6.4, §7.1 et §7.3. Rien de déployé : les migrations
011 et 014 se jouent à la main en production comme les précédentes.

| Route (`app.runeride.fr`, Bearer de session) | Rôle |
|---|---|
| `POST /api/balade/jeton` `{"type": "petite_boucle" \| "grand"}` | jeton court **RS256** signé par la même clé que les sessions (`Jwt::issueFor()`), `aud = raido`, `iss = RUNERIDE_BASE_URL`, `exp` 15 min, `sub` opaque (`u` + 24 hexadécimaux de SHA-256, jamais l'identifiant du compte), revendications `palier`, `type`, `limite_km` (25). Un **grand** calcul est décompté à la délivrance pour un compte gratuit : **3 par mois civil** (`403 quota_epuise` ensuite ; `UPDATE` conditionnel, atomique), jamais pour un abonné. Réponse : `jeton`, `expire_a`, `raido_url` (`RUNERIDE_RAIDO_URL`, vide tant que rien n'est déployé), restant du mois |
| `GET /api/balade/quota` | restant du mois, pour l'affichage |
| `GET /api/balades`, `GET /api/balade?id=`, `POST /api/balades`, `DELETE /api/balades?id=` | balades enregistrées (migration 011 : `params_json` = requête §7.2, `trace_json` = polyligne encodée, mesures, scores, version du graphe) ; **5 en gratuit** (`403 abonnement_requis`), illimité en abonné |
| `PUT /api/balade/partage {id}`, `DELETE /api/balade/partage?id=` | crée (clé aléatoire stable) ou révoque le lien public |
| `GET /api/balade/partage?cle=` (sans jeton) | titre, tracé, mesures, scores, date, `attribution` ; ni compte ni paramètres de calcul |

Variables du `.env` (toutes avec valeur par défaut) : `RUNERIDE_RAIDO_AUDIENCE` (`raido`), `RUNERIDE_RAIDO_JETON_TTL` (900),
`RUNERIDE_RAIDO_URL`, `RUNERIDE_BALADE_LIMITE_KM` (25), `RUNERIDE_BALADE_GRANDS_PAR_MOIS` (3), `RUNERIDE_BALADE_ENREGISTREES_GRATUIT` (5).
Côté fonction, `RAIDO_EMETTEUR` doit valoir le `RUNERIDE_BASE_URL` du PHP (`https://app.runeride.fr`, défaut du module Terraform) et `RAIDO_JWKS_URL` son `/.well-known/jwks.json`.
La constante d'attribution est identique dans `BaladeApi::ATTRIBUTION` et `raido_calcul::requete::ATTRIBUTION`.

### Préférences du Compte lues par le moteur (19/10/2026 au soir)

`POST /v1/balade` et `/v1/trajet` acceptent `vitesses` (objet classe → km/h, classes `autoroute`, `nationale`, `primaire`,
`secondaire`, `tertiaire`, `non_classee`, `residentielle`, 20 à 130 km/h) et `pause_min_par_h` (0 à 30, défaut 10), les
valeurs que la PWA lit dans `GET /api/balade/preferences`. Le temps de chaque arc est multiplié par
`vitesse du graphe ÷ vitesse réglée` (`Preferences::echelle_temps`), dans le **coût** comme dans la **durée rendue** ; la pause
sert aussi à convertir une durée demandée en distance. Les vitesses réglées sont **ramenées à 110 km/h** : c'est ce qui garde
le temps à vol d'oiseau à 110 km/h admissible pour A\*. Limite : le plafond `maxspeed` déjà appliqué dans le temps de base du
graphe n'est pas relevé (réglage à la hausse sur une route limitée : sans effet), et la conversion durée → distance reste à 55 km/h.
Les seuils météo ne sont pas lus (météo : jalon J5). Classe inconnue ou valeur hors bornes : `422 calcul_impossible`.

Mesuré sur le graphe Rhône-Alpes, boucle de 60 km depuis Grenoble, mêmes préférences de caractère :

| Requête | Distance | Durée rendue |
|---|---|---|
| défauts | 60,3 km | 5 314 s |
| vitesses réduites (nationale 45 … résidentielle 20 km/h) | 60,5 km | 8 956 s |
| `pause_min_par_h` = 0 | 60,3 km | 4 555 s |
| `pause_min_par_h` = 30 | 60,3 km | 6 833 s |

### Appariement des tracés au réseau, `POST /v1/apparier` (20/10/2026)

Rattache un tracé GPS aux tronçons du graphe, pour remplir `trajets_troncons` (migration 012) et alimenter la découverte (spec §4.4).
Module `raido-calcul/src/apparier.rs` : le tracé est échantillonné (un point tous les 15 m au plus), chaque point retient les
tronçons à moins de **50 m**, puis un Viterbi choisit la suite la plus plausible (coût = distance² à σ = 15 m ; même tronçon 0,
tronçon voisin 0,5, tronçon éloigné 3). Un tronçon effleuré par un seul point n'est pas retenu.

| | |
|---|---|
| Entrée | `{"trace": [[lat, lon], …]}` (au moins deux points distants de 15 m ; 60 000 points échantillonnés et 600 km d'emprise au plus) |
| Sortie | `troncons` `[[way_id, segment], …]` triés et distincts, `couverture` (0 à 1), `points`, `points_apparies`, `graphe.version` |
| Accès | **privé** : en-tête `x-cle-service` égal à `RAIDO_CLE_SERVICE` (comparaison à temps constant). Variable absente : la route n'existe pas (404). Clé fausse : 401 `cle_invalide`. Trace invalide : 422 `apparier_impossible` |

Le PHP stocke la réponse avec la `graphe.version` reçue : les identifiants ne sont comparables que pour une même version.
Ni la route ni le secret ne sont dans le module Terraform : `RAIDO_CLE_SERVICE` doit venir d'un secret (Secrets Manager),
décision à prendre avant le premier `apply`.

**Mesuré** (`raido-calcul -g <graphe> apparier`) : boucles de 60 km calculées sur Rhône-Alpes, rejouées avec un bruit gaussien
sur chaque point, comparées aux tronçons réellement empruntés (rappel pondéré par la longueur) ; 12 boucles par ligne.

| Bruit GPS (σ) | Rappel | Précision | Couverture | Temps par trace |
|---|---|---|---|---|
| 3 m | 92,0 % | 99,6 % | 99,6 % | 0,05 s |
| 8 m | 92,9 % | 98,3 % | 99,7 % | 0,06 s |
| 20 m | 92,3 % | 85,3 % | 98,8 % | 0,07 s |

Le rappel plafonne vers 92 % quel que soit le bruit : ce sont surtout des tronçons très courts (raccords, ronds-points) que le
filtre « au moins deux points » écarte ; ils pèsent peu pour la découverte. **Limites** : la vérité de l'essai vient du même
graphe que l'appariement (tracés idéaux hors bruit) ; **aucun tracé GeoRide réel** n'a été apparié (rien de tel sur ce poste) ;
tronçons de plus de 1,5 km sans intersection vus seulement si un de leurs nœuds est proche ; pas d'horodatage ni de vitesse.
L'appel après `POST /api/georide/synchronisation` (PHP) reste à écrire.

### Distance des petites boucles avec découverte (21/10/2026)

Constat : avec l'historique d'une boucle précédente et une forte découverte, les boucles de 25 km pouvaient s'écarter de la
cible (jusqu'à +59 %). Le raffinement du rayon des pétales, jusque-là **une seule** correction des trois meilleurs candidats,
fait désormais **jusqu'à trois passes** (`PASSES_RAFFINEMENT`), arrêtées dès qu'aucune ne s'améliore : les pénalités rendent la
distance peu proportionnelle au rayon.

Mesuré sur Rhône-Alpes (départs tirés au hasard, graine fixe, historique = tronçons appariés d'une boucle précédente) :

| Boucle | Découverte | Départs hors ±15 % avant → après | \|écart\| moyen avant → après | Nouveauté moyenne après |
|---|---|---|---|---|
| 25 km, 10 départs | 0 | 1 → 1 | 7,8 → 7,2 % | 4 % |
| | 2 | 2 → 2 | 12,7 → 10,3 % | 67 % |
| | 3 | 4 → 2 | 14,3 → 10,1 % | 78 % |
| | 4 | 4 → 2 | 15,5 → 11,4 % | 76 % |
| 60 km, 8 départs | 0 à 4 | 0 → 0 à 1 | 3,7 à 6,0 → 3,1 à 4,6 % | 73 à 76 % |

**Reste** : un départ sur dix reste à +35 % à +59 % à 25 km, y compris sans découverte (fond de vallée sans boucle courte) ; la réponse
porte alors l'avertissement « distance obtenue à +N % de la cible ». Le temps de calcul ne change pas de façon mesurable
(≈ 0,1 à 0,5 s par boucle de 25 km). Écart d'un départ sur le 60 km à découverte 3 : 15,2 % (1 sur 8), 7 % avant.

### Essai de bout en bout, sans AWS

`rust/tools/essai-jeton-local.sh <runeride/web> <dossier du graphe>` : la classe PHP `Jwt` de `runeride` engendre une
clé jetable, publie son JWKS et signe six jetons exactement comme `BaladeApi::token()` ; `bootstrap --evenement` rejoue
13 événements Function URL avec `RAIDO_JETON=obligatoire` et `RAIDO_JWKS_FICHIER`. Rejoué le 19/10/2026 sur le graphe
Rhône-Alpes z9 (25 tuiles), départ Grenoble, 4 fils ; les 13 statuts sont ceux attendus (`essai-jeton/cas.py`) :

| Cas | Statut | Code | Détail |
|---|---|---|---|
| `01_sante_sans_jeton` | 200 | | `GET /v1/sante` reste public |
| `02_balade_sans_jeton` | 401 | `jeton_requis` | |
| `03_session_runeride` | 401 | `jeton_invalide` | jeton de session (`aud = runeride`) refusé |
| `04_expire` | 401 | `jeton_invalide` | expiré depuis 59 min |
| `05_autre_cle` | 401 | `jeton_invalide` | `kid` inconnu du JWKS |
| `06_gratuit_petite_20km` | 200 | | 22,2 km, 79 ms |
| `07_gratuit_petite_40km` | 403 | `quota_requis` | distance > `limite_km` |
| `08_gratuit_petite_180min` | 403 | `quota_requis` | durée qui implique > 25 km |
| `09_gratuit_petite_trajet` | 403 | `quota_requis` | `/v1/trajet` interdit en petite boucle |
| `10_gratuit_grand_120km` | 200 | | 105,8 km, 440 ms |
| `11_gratuit_grand_decouverte` | 403 | `abonnement_requis` | découverte en gratuit |
| `12_abonne_grand_decouverte` | 200 | | 58,8 km, 259 ms |
| `13_abonne_trajet` | 200 | | Grenoble → Lyon 105,0 km, 81 ms |

**Écart avec la spec §6.4 à trancher** (« n'est pas consommé si le calcul échoue ») : le PHP décompte à la délivrance
et ne sait pas si le calcul a abouti ; Raido est sans état et n'appelle pas le PHP. Un remboursement sur déclaration du
client serait contournable (quota illimité). Deux issues : accepter l'écart en v1 (la PWA vérifie les bornes avant de
demander le jeton, un `422 calcul_impossible` reste rare), ou faire remonter l'échec par Raido (appel sortant signé
vers le PHP), hors v1. Non tranché au 19/10/2026.

Ce que cet essai ne montre pas : le téléchargement du JWKS par HTTPS (`RAIDO_JWKS_URL`, pile TLS en Rust pur, non
exercée contre un vrai serveur), le `POST` derrière CloudFront (`x-amz-content-sha256`), les temps réels sur arm64.

### Plan d'essai sur AWS (à faire, chaque étape sur accord explicite, rien d'exécuté au 21/10/2026)

**Préparé en local, sans AWS** : zip arm64 reconstruit le 21/10/2026 par `rust/tools/package-lambda.sh` (1 361 588 octets,
SHA-256 `d9d30aa438ed409fe8e7c3bc07f98ab8af34a033106c33b99da186ad2e199e78`, binaire 2,6 Mo), contenant la route privée `/v1/apparier`, les
préférences et les passes de raffinement. **Non exécuté sur arm64** : aucune émulation sur ce poste, seul x86_64 a tourné (le doute
sur le libm de musl arm64, ±1 code, reste à lever sur Graviton). Module Terraform : `fmt`, `init -backend=false` et `validate` OK, avec la
variable sensible `cle_service` (`runeride-cloud` #4).

**Décisions et réglages à avoir en main avant le premier `apply`**

| Sujet | Valeur proposée |
|---|---|
| Clé de service | 32 caractères au moins (`openssl rand -base64 36`), dans `TF_VAR_cle_service` et dans `RUNERIDE_RAIDO_CLE_SERVICE` côté application, jamais dans Git ; sans clé, la route `/v1/apparier` n'existe pas et la découverte reste inactive |
| Émetteur du jeton | `jeton_emetteur = https://app.runeride.fr` (= `RUNERIDE_BASE_URL` de l'application), `jeton_audience = raido` |
| Adresse de Raido côté application | `RUNERIDE_RAIDO_URL=https://raido.runeride.fr` **seulement après** le rattachement du domaine ; avant cela la PWA dit que le calculateur n'est pas en service |
| Migrations de l'application | 011 à 016 à jouer à la main (`ops/app-provision.sh --migrate` passe par l'API2 Cron) avant d'ouvrir la vue Balade |
| Coût | budget d'alerte de 5 $ du module, concurrence réservée à 10 dès que le quota du compte le permet ; **ESTIMATION < 1 $ par mois** (spec §5.5), non mesurée |

**Étapes**

1. Accord de Philippe, identifiants AWS du projet, état Terraform (`backend "s3"` partiel : le compartiment d'état doit exister).
2. `terraform plan` du module `runeride-cloud/terraform/raido/` (à lire en entier : première fois), avec `jeton_emetteur = https://app.runeride.fr`.
3. `terraform apply` sans le domaine, `publier_graphe`, `GET /v1/sante` sur l'adresse CloudFront : vérifie le TLS vers S3 **et** le chargement du JWKS depuis `app.runeride.fr` (journal `{"evenement":"jwks","cles":1}`).
4. Depuis un compte RuneRide de test : `POST /api/balade/jeton` puis le `POST /v1/balade` du README du module avec `x-amz-content-sha256` ; rejouer les cas 06, 07 et 11 du tableau ci-dessus ; puis une synchronisation GeoRide d'un abonné de test et le journal de `/v1/apparier` (clé, `x-amz-content-sha256`).
5. Mesurer démarrage à froid, `MaxMemoryUsed`, coût sur deux semaines ; comparer aux ESTIMATIONS de la spec §5.5.
6. Seulement ensuite : `certificat_valide = true`, CNAME `raido`, `RUNERIDE_RAIDO_URL` dans le `.env` de production.

## 2.7 Jalon J2 : pipeline du score scénique (21/10/2026)

**Fait** : l'extraction et le calcul. **Pas fait** : la calibration, qui demande tes 10 balades de référence (spec §10, J2). Les poids ci-dessous sont
**provisoires**.

### Ce que le pipeline lit (`raido-graphe construire`)

| Source OSM | Rôle | Rhône-Alpes (extrait du 18/10) |
|---|---|---|
| nœuds `mountain_pass=yes`, ou `natural=saddle` nommé « Col … », « Cormet … », « Pas … » | cols | 2 029 cols nommés |
| nœuds `tourism=viewpoint` ; `natural=peak` | panoramas ; sommets | 810 ; 3 923 nommés |
| ways `natural=water` ; `landuse=forest` ou `natural=wood` ; `leisure=nature_reserve`, `boundary=national_park` ou `protected_area` | eau, forêts, parcs : contour échantillonné tous les 120 m | 666 803 points et échantillons au total |
| ways `landuse=industrial\|commercial\|retail` | **pénalité** | idem |
| relations `type=route` avec `route=motorbike`, ou `route=road` avec `tourism` ou `scenic=yes` | routes touristiques : bonus à leurs ways | 4 078 ways |

Score d'un tronçon (0 à 1) : échantillons le long du tronçon (un tous les 150 m) ; pour chaque genre, part des échantillons à moins de **300 m**
d'un point de ce genre ; bonus = 1 − Π(1 − poids × part), relevé par l'appartenance à une route touristique, moins la pénalité de zone d'activité.
Poids par défaut : col 0,9 ; panorama 0,7 ; sommet 0,4 ; eau 0,35 ; forêt 0,3 ; parc 0,3 ; route touristique 0,5 ; activité −0,5.
**Réglage sans recompiler** : `raido-graphe construire … --poids poids.json` (champs facultatifs, une faute de frappe est refusée) ;
`--dump-poi fichier.geojson` écrit les points nommés pour vérifier à la main. Format du graphe inchangé (version 3) : un nouveau graphe se
publie tel quel, sans toucher à la Lambda.

Mesuré sur Rhône-Alpes : construction **8,5 s**, 1,7 Go de mémoire ; **33,1 %** des arcs ont un score positif, moyenne 0,113. Galibier, Iseran,
Glandon, Madeleine, Télégraphe, Roselend et Col de Porte sont présents. Le Lautaret et le Granon n'y sont pas : ils sont **hors de l'extrait**
(région PACA).

### Dans le moteur

Le classement des 24 candidats de boucle tient désormais compte du score scénique moyen, proportionnellement au curseur « scénique »
(jusqu'à 3 points de note à 4/4, comme la sinuosité l'était déjà).

### Ce que la mesure montre, honnêtement

- **Avec les seuls pétales, le curseur changeait peu les boucles** (corrigé par les points de passage ci-dessous). Six départs alpins, boucles de 120 km, scénique 0 contre 4 : score scénique moyen 0,23 → 0,27 ; sur le
  même graphe, 97,8 % de la boucle à 100 m près est commune. La cause est structurelle : une boucle est bâtie sur trois points de passage posés
  sur un triangle selon un cap ; entre ces points A\* n'a que peu d'alternatives. Les 24 candidats se ressemblent et leurs scores scéniques
  diffèrent peu. D'où les candidats bâtis sur un col ou un panorama (section suivante).
- **Distance des boucles** : sur 24 départs, 25 km, nouveau graphe contre ancien : |écart| moyen 6,1 → 7,2 % à découverte 0, 9,4 → 9,2 % à 2, 10,1 → 9,9 % à 4 ;
  60 km (12 départs) 3,4 → 4,1 % à 0. Dans le bruit de la mesure, avec une légère tendance à la hausse à 25 km sans découverte.
- **Limites de l'extraction** : les relations `multipolygon` (grandes forêts, parcs nationaux souvent saisis ainsi) ne sont pas lues, seuls les ways
  étiquetés directement le sont ; seuls les contours comptent (une route à l'intérieur d'une grande forêt n'en profite pas) ; **pas de relief** : la
  BD ALTI de l'IGN n'est pas encore intégrée, donc ni dénivelé relatif ni bonus « route de montagne ».

### Points de passage sur les cols et panoramas (21/10/2026)

Réponse au constat ci-dessus. Le constructeur écrit `attracteurs.json` à côté de `index.json` : les **cols et panoramas nommés** de l'emprise
(2 768 sur Rhône-Alpes, 201 Ko, dédoublonnés à 100 m, triés). Le moteur le charge s'il existe (un graphe plus ancien marche comme avant).
Avec le curseur « scénique » à **2 ou plus**, `boucle::balade` ajoute des candidats bâtis sur un attracteur : au plus 8, à une distance du
départ comprise entre 0,6 et 1,5 fois le rayon nominal des pétales, cols avant panoramas, sans deux attracteurs à moins de 1 km ; chacun dans
les deux sens du triangle (± 60°). Le premier point de passage est le col, le second ferme le triangle ; le rayon du second est **corrigé jusqu'à
trois fois** pour tenir la distance, comme pour les pétales. Un candidat attracteur reçoit un bonus de note de 0,4 × le poids du curseur
(0,4 à 2/4, 0,8 à 4/4), **seulement si sa distance reste dans la tolérance** (sinon les boucles dépassaient de 20 %). La réponse porte
`passe_par` : le col ou le panorama choisi (`null` pour une boucle en pétale).

Mesuré sur Rhône-Alpes, six départs, boucles de 120 km, scénique 0 contre 4 : **les boucles changent vraiment** (Annecy : Tamié, Croix Fry et
Aravis ; Bourg-Saint-Maurice : Roselend ; Grenoble : Granier, Placette) ; score scénique moyen 0,23 → 0,28, cols traversés 3,6 → 4,2 par boucle.
Distance (écart moyen absolu, scénique 0 / 2 / 3 / 4) : **25 km, 24 départs** 6,9 / 7,9 / 10,4 / 10,3 % (6,1 à 7,2 % avant tout ce travail, à
découverte 0) ; **60 km, 12 départs** 4,6 / 3,8 / 4,7 / 4,9 %, un départ à 22,7 % pour les curseurs 3 et 4. **Coût** : banc de 12 boucles de
120 km, médiane **0,42 s → 0,58 s** (p95 0,47 → 0,73 s) quand le scénique est demandé ; rien ne change pour un curseur à 0 ou 1.

Limites : le score scénique reste un indicateur grossier (« plus de cols » n'est pas « plus beau ») : **seules tes balades de référence diront si
ces boucles sont les bonnes**. Un col dont le nœud OSM est loin d'une route carrossable n'est pas utilisable (accroche à 800 m). La PWA n'affiche
pas encore `passe_par`.

### Calibrer avec tes balades : `rust/tools/evaluer-references.py`

```sh
# un dossier de fichiers .gpx (une trace chacun), un graphe construit ; essaie des jeux de poids (chacun reconstruit un graphe, ≈ 9 s)
python3 -I rust/tools/evaluer-references.py ~/references donnees/graphe/rhone-alpes --poids a.json b.json --scenique 4
```

Pour chaque référence, le premier point sert de départ et la longueur de distance cible ; Raido calcule les 3 premières variantes ; la référence
« sort » si l'une couvre au moins **70 %** de sa longueur à moins de **100 m** (`--seuil`, `--rayon`). Critère de la spec : 7 références sur 10.
**Ces deux seuils sont ma lecture du critère**, à valider avec toi. Essai de l'outil avec des références synthétiques : deux boucles retrouvées à
99 et 100 %, une trace décalée de 2 km à 4 % (non retrouvée), une boucle recalculée à une distance un peu différente à 67 %.

## 3. Ce qui n'a pas été fait, honnêtement

- **Aucune mesure sur AWS** : ni Lambda, ni S3 réel (le chargement « à froid » est mesuré sur disque local et sur un faux S3 en boucle locale, §2.5 ; S3 en région ajoutera de l'ordre de 0,5 à 1 s pour 54 Mo : ESTIMATION). Le packaging arm64 est fait (§2.5), pas exécuté sur arm64.
- Scores **scénique** et **motard** à zéro, pas de relief (BD ALTI), pas de relations touristiques, pas de cols ni de panoramas.
- Pas de graphe grossier pour les longs trajets (spec §5.3) : Lyon → Grenoble passe très bien sans, la question se posera pour des trajets > 300 km.
- Restrictions avec pivot « way » ignorées (13 %) ; pas de ferries ; `access` conditionnel non lu ; pas de prise en compte de `maxweight`/`maxheight` (sans objet à moto).
- Plausibilité des tracés : **validée par le titulaire le 18/10/2026** sur la visionneuse HTML (Leaflet + fond OSM) pour les boucles depuis Grenoble, Lyon et Annecy et le trajet Lyon → Grenoble ; pas encore de comparaison chiffrée avec des balades de référence (J2).
- Pas d'étapes, d'autonomie carburant, de météo ni de découverte par référence S3 dans le moteur : jalons J4 à J6 (la découverte par `troncons_connus` dans le corps de la requête existe). Côté PHP, J4 a livré le jeton, les quotas et les balades enregistrées (§2.6) ; la vue PWA `#/balade` reste à faire.
- Pas de test d'intégration sur un graphe jouet (seulement 13 tests unitaires sur le format, l'extraction et les scores).
- Une seule machine, un seul extrait, caches disque chauds pour les bancs « à chaud ».
