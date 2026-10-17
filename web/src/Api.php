<?php
declare(strict_types=1);

namespace Carto;

use Carto\Http\{ApiException, Validation};
use Carto\Repository\{LieuRepository, StationRepository};
use Doctrine\DBAL\Connection;

/** Routes /v1 : transforme requêtes validées + dépôts en réponses JSON. */
final class Api
{
    public const ATTRIBUTION = "© les contributeurs d'OpenStreetMap (ODbL) https://www.openstreetmap.org/copyright";

    /** @param callable():Connection $pdo connexion paresseuse : /v1/sante ne touche pas la base inutilement */
    public static function register(Router $router, callable $pdo, string $env, array &$query): void
    {
        $router->add('GET', '/v1/sante', function () use ($pdo, $env) {
            $pdo()->executeQuery('SELECT 1')->fetchOne();
            return ['statut' => 'ok', 'env' => $env, 'version' => 'v1'];
        });

        $router->add('GET', '/v1/stations', function () use ($pdo, &$query) {
            $lat = Validation::latitude($query);
            $lon = Validation::longitude($query);
            $rayon = Validation::rayonKm($query);
            $marque = Validation::marque($query);
            $limite = Validation::limite($query, 50);
            $zoom = Validation::zoom($query);
            $rows = (new StationRepository($pdo()))->autour($lat, $lon, $rayon * 1000.0, $marque, $limite, $zoom);
            return self::enveloppe(array_map(self::station(...), $rows), $limite);
        });

        $router->add('GET', '/v1/lieux', function () use ($pdo, &$query) {
            $q = Validation::prefixe($query);
            $limite = Validation::limite($query, 20);
            $lat = Validation::latitude($query, 'lat', false);
            $lon = Validation::longitude($query, 'lon', false);
            if (($lat === null) !== ($lon === null)) {
                throw new ApiException(400, 'lat et lon vont ensemble', $lat === null ? 'lat' : 'lon');
            }
            $zoom = Validation::zoom($query);
            $rows = (new LieuRepository($pdo()))->parPrefixe($q, $limite, $lat, $lon, $zoom);
            return self::enveloppe(array_map(self::lieu(...), $rows), $limite);
        });

        // Points visibles dans une boîte à un zoom donné : stations et lieux, limite totale bornée.
        $router->add('GET', '/v1/vue', function () use ($pdo, &$query) {
            $zoom = Validation::zoom($query, true);
            $bbox = Validation::bbox($query, $zoom);
            $limite = Validation::limite($query, Validation::VUE_LIMITE_DEFAUT, Validation::VUE_LIMITE_MAX);
            $db = $pdo();
            // limite + 1 de chaque côté suffit à savoir s'il y a troncature, quelle que soit la répartition
            $items = [];
            foreach ((new StationRepository($db))->visibles($bbox, $zoom, $limite + 1) as $r) {
                $items[] = ['s', (int) $r['min_zoom'], 0, $r];
            }
            foreach ((new LieuRepository($db))->visibles($bbox, $zoom, $limite + 1) as $r) {
                $items[] = ['l', (int) $r['min_zoom'], (int) ($r['population'] ?? 0), $r];
            }
            // les plus importants d'abord : min_zoom croissant, puis population décroissante (tri stable)
            usort($items, fn ($a, $b) => [$a[1], -$a[2]] <=> [$b[1], -$b[2]]);
            $tronque = count($items) > $limite;
            $items = array_slice($items, 0, $limite);
            $stations = $lieux = [];
            foreach ($items as [$type, , , $r]) {
                if ($type === 's') {
                    $stations[] = self::station($r);
                } else {
                    $lieux[] = self::lieu($r);
                }
            }
            return [
                'donnees' => ['stations' => $stations, 'lieux' => $lieux],
                'meta' => ['nombre' => count($items), 'limite' => $limite, 'tronque' => $tronque, 'zoom' => $zoom, 'bbox' => $bbox],
                'attribution' => self::ATTRIBUTION,
            ];
        });
    }

    /** @param list<array<string,mixed>> $donnees */
    private static function enveloppe(array $donnees, int $limite): array
    {
        return ['donnees' => $donnees, 'meta' => ['nombre' => count($donnees), 'limite' => $limite], 'attribution' => self::ATTRIBUTION];
    }

    /** @param array<string,mixed> $r */
    private static function station(array $r): array
    {
        $out = [
            'id' => $r['osm_type'] . '/' . $r['osm_id'],
            'nom' => $r['nom'],
            'marque' => $r['marque'],
            'exploitant' => $r['exploitant'],
            'carburants' => $r['carburants'] === null || $r['carburants'] === '' ? [] : explode(',', (string) $r['carburants']),
            'adresse' => (object) array_filter([
                'numero' => $r['adr_numero'], 'rue' => $r['adr_rue'], 'code_postal' => $r['adr_cp'], 'ville' => $r['adr_ville'],
            ], fn ($v) => $v !== null),
            'lat' => (float) $r['lat'],
            'lon' => (float) $r['lon'],
            'min_zoom' => (int) $r['min_zoom'],
        ];
        if (isset($r['distance_m'])) {
            $out['distance_m'] = (int) round((float) $r['distance_m']);
        }
        return $out;
    }

    /** @param array<string,mixed> $r */
    private static function lieu(array $r): array
    {
        $out = [
            'id' => $r['osm_type'] . '/' . $r['osm_id'],
            'nom' => $r['nom'],
            'nom_alt' => $r['nom_alt'],
            'categorie' => $r['categorie'],
            'population' => $r['population'] === null ? null : (int) $r['population'],
            'lat' => (float) $r['lat'],
            'lon' => (float) $r['lon'],
            'min_zoom' => (int) $r['min_zoom'],
        ];
        if (isset($r['distance_m'])) {
            $out['distance_m'] = (int) round((float) $r['distance_m']);
        }
        return $out;
    }
}
