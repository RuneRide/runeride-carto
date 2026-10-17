# Fond de carte en tuiles vectorielles PMTiles (preuve de concept)

Statut : PoC. Rien n'est déployé. (Repris du dépôt `runeride` ; `FOND_DE_CARTE` de l'application y reste à brancher.)
Démo : `web/public/tuiles-demo.html`.

## Architecture

- Un seul fichier `.pmtiles` (extrait Protomaps, données OpenStreetMap sous ODbL), posé sur le mutualisé comme un fichier statique.
- Le navigateur le lit par requêtes HTTP `Range` (pmtiles.js), le rendu est fait côté client par MapLibre GL JS avec le style `@protomaps/basemaps` (thèmes clair et sombre). Aucun PHP, aucun processus persistant.
- Dépendances en CDN avec SRI et versions épinglées : maplibre-gl 5.24.0 (le 6.x n'est plus livré en UMD), pmtiles 4.5.0, @protomaps/basemaps 5.7.2. Les polices (glyphs) et sprites viennent de `protomaps.github.io/basemaps-assets` : à copier chez nous pour ne dépendre de rien (zips dans le dépôt `basemaps-assets`).
- Attribution obligatoire : « Protomaps © OpenStreetMap contributors » (déjà dans la source du style de la démo).
- `web/public/.htaccess` : type MIME `application/vnd.pmtiles`, pas de compression, cache. Non testé sur le vrai Apache.
- Emplacement des fichiers : `web/storage/tuiles/` est ignoré par git (`web/.gitignore` : `storage/*`). La démo lit `/tuiles/toulouse.pmtiles` par défaut, ou `?src=URL`. Sur o2switch, le fichier devra être sous `public/app/tuiles/` (ou autre dossier public) : à décider, l'URL se règle par `?src=`.

## Générer / mettre à jour

Outil : binaire unique `pmtiles` (go-pmtiles, v1.31.2 testée), à télécharger depuis https://github.com/protomaps/go-pmtiles/releases (archive `go-pmtiles_<v>_Linux_x86_64.tar.gz`). Aucune somme de contrôle n'est publiée avec la release (le sha256 de l'archive téléchargée était `3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f`, relevé pour mémoire, non comparé à une référence officielle).

Builds quotidiens du planète (~139 Go, zooms 0-15, hashes BLAKE3 sur https://maps.protomaps.com/builds) : `https://build.protomaps.com/AAAAMMJJ.pmtiles`. `pmtiles extract` lit à distance seulement les plages utiles, sans télécharger les 139 Go.

```sh
pmtiles extract https://build.protomaps.com/20261016.pmtiles web/storage/tuiles/toulouse.pmtiles \
  --bbox=-0.26,42.35,3.14,44.85 --maxzoom=15
```

Mise à jour : relancer avec le build du jour (la date change chaque jour ; prendre la dernière avec HTTP 200) et remplacer le fichier (envoyer sous un autre nom puis renommer, pour ne pas servir un fichier partiel). Rythme raisonnable : trimestriel.

## Poids

| Extrait | Zooms | Taille | Temps |
|---|---|---|---|
| bbox ~273 x 278 km (~76 000 km2) autour de Toulouse, Occitanie ouest, un peu d'Espagne et de mer | 0-15 | **656 Mo (mesuré)** | 42 s, 688 Mo transférés |
| France métropolitaine (~551 000 km2) | 0-15 | **~4,7 Go (ESTIMATION)**, fourchette 3,5-6 Go | non mesuré |

L'estimation est une simple règle de trois sur la surface (~8,6 Mo / 1000 km2). Elle est grossière : la densité varie fortement (villes, bâti, relief, mer). À mesurer en vrai avant décision. La doc Protomaps indique que chaque zoom en moins réduit la taille (jusqu'à environ moitié) ; `--maxzoom=14` ou 13 est une piste si le quota pose problème.

## Ce qui a été testé

Page de démo + fichier de 656 Mo servis en local par un petit serveur Node gérant `Range`, rendu dans Chrome headless (CDP) : carte clair et sombre affichée sur Toulouse, 8 requêtes sur le fichier, toutes en 206. Le serveur intégré `php -S` répond 200 à une requête `Range` (ignorée) : inutilisable pour ce test, comme `python3 -m http.server`. Non testé : Apache réel, o2switch, mobile, charge, hors-ligne/service worker, intégration à Leaflet/à l'application.

## Risques sur mutualisé (à confirmer avec o2switch)

- Quota disque / inodes : un fichier de 5 Go seulement une entrée, mais compter les sauvegardes et les mises à jour (deux versions en même temps).
- Limite de taille de fichier à l'envoi (FTP/cPanel) et durée max de transfert pour 5 Go.
- Bande passante : une carte vue = quelques dizaines à centaines de Ko, mais le trafic « illimité » est soumis à un usage raisonnable ; la charge disque (I/O) sur de nombreuses requêtes Range simultanées est le vrai point d'attention.
- Cache/CDN : un CDN devant (Cloudflare) peut ne pas supporter Range sur de gros fichiers ; à vérifier.
- Vérifier que `.htaccess` est pris en compte (AllowOverride) et que mod_deflate ne recompresse pas.

## Alternative : Stadia Maps

Offre hébergée, sans fichier à gérer. Plan Starter à 20 $/mois selon un comparatif consulté (à reconfirmer sur le site du fournisseur avant tout engagement, ainsi que l'usage commercial autorisé et l'attribution). Possibilité de commencer ainsi puis de basculer sur PMTiles.
