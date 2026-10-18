<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Raido : graphe routier et moteur de balade (PoC J1)

Statut : **jalon J1 de [runeride/docs/itineraire.md](../../runeride/docs/itineraire.md) réalisé et accepté le 18/10/2026** (tracés validés par le titulaire), tout en local, rien n'est déployé. Ce document décrit ce qui existe dans `rust/`, les mesures obtenues sur l'extrait Rhône-Alpes et ce qui n'a pas été fait. Vocabulaire : **mesuré** / **ESTIMATION** / **OUVERT**.

## 1. Ce qui existe

Espace de travail Cargo `rust/` (même organisation que `runeride-weather/rust`), trois caisses sous EUPL-1.2 :

| Caisse | Rôle | Fichiers |
|---|---|---|
| `raido-format` | types des tuiles de graphe (`rkyv` 0.8), identifiants globaux, découpage z8, haversine, géohash | `crates/raido-format/src/lib.rs` |
| `raido-graphe` | pipeline : lecture du PBF (`osmpbf`, deux passes parallèles), découpage aux intersections, scores, restrictions de tourner, composantes connexes, écriture des tuiles et de `index.json` | `osm.rs`, `scores.rs`, `construire.rs`, `main.rs` |
| `raido-calcul` | moteur : chargement des tuiles à la demande (validation rkyv au chargement, accès zéro copie ensuite), fonction de coût, A\* par arcs avec restrictions et interdiction de demi-tour, boucles par pétales parallélisées (`rayon`), ligne de commande `trajet` / `balade` / `banc` | `charge.rs`, `cout.rs`, `astar.rs`, `boucle.rs`, `geo.rs`, `main.rs` |

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

## 3. Ce qui n'a pas été fait, honnêtement

- **Aucune mesure sur AWS** : ni Lambda, ni S3 (le chargement « à froid » est mesuré sur disque local, S3 en région ajoutera de l'ordre de 0,5 à 1 s pour 76 Mo : ESTIMATION). Ni packaging arm64.
- Scores **scénique** et **motard** à zéro, pas de relief (BD ALTI), pas de relations touristiques, pas de cols ni de panoramas.
- Pas de graphe grossier pour les longs trajets (spec §5.3) : Lyon → Grenoble passe très bien sans, la question se posera pour des trajets > 300 km.
- Restrictions avec pivot « way » ignorées (13 %) ; pas de ferries ; `access` conditionnel non lu ; pas de prise en compte de `maxweight`/`maxheight` (sans objet à moto).
- Plausibilité des tracés : **validée par le titulaire le 18/10/2026** sur la visionneuse HTML (Leaflet + fond OSM) pour les boucles depuis Grenoble, Lyon et Annecy et le trajet Lyon → Grenoble ; pas encore de comparaison chiffrée avec des balades de référence (J2).
- Pas d'étapes, d'autonomie carburant, de météo ni de découverte dans le moteur : ce sont les jalons J3, J5 et J6.
- Pas de test d'intégration sur un graphe jouet (seulement 13 tests unitaires sur le format, l'extraction et les scores).
- Une seule machine, un seul extrait, caches disque chauds pour les bancs « à chaud ».
