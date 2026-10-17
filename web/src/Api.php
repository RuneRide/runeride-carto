<?php
declare(strict_types=1);

namespace Carto;

use Carto\Http\{ApiException, Validation};
use Carto\Repository\{LieuRepository, StationRepository};
use PDO;

/** Routes /v1 : transforme requêtes validées + dépôts en réponses JSON. */
final class Api
{
    public const ATTRIBUTION = "© les contributeurs d'OpenStreetMap (ODbL) https://www.openstreetmap.org/copyright";

    /** @param callable():PDO $pdo connexion paresseuse : /v1/sante ne touche pas la base inutilement */
    public static function register(Router $router, callable $pdo, string $env, array &$query): void
    {
        $router->add('GET', '/v1/sante', function () use ($pdo, $env) {
            $pdo()->query('SELECT 1')->fetchColumn();
            return ['statut' => 'ok', 'env' => $env, 'version' => 'v1'];
        });

        $router->add('GET', '/v1/stations', function () use ($pdo, &$query) {
            $lat = Validation::latitude($query);
            $lon = Validation::longitude($query);
            $rayon = Validation::rayonKm($query);
            $marque = Validation::marque($query);
            $limite = Validation::limite($query, 50);
            $rows = (new StationRepository($pdo()))->autour($lat, $lon, $rayon * 1000.0, $marque, $limite);
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
            $rows = (new LieuRepository($pdo()))->parPrefixe($q, $limite, $lat, $lon);
            return self::enveloppe(array_map(self::lieu(...), $rows), $limite);
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
        return [
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
            'distance_m' => (int) round((float) $r['distance_m']),
        ];
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
        ];
        if (isset($r['distance_m'])) {
            $out['distance_m'] = (int) round((float) $r['distance_m']);
        }
        return $out;
    }
}
