<?php
declare(strict_types=1);

namespace Carto\Repository;

use Doctrine\DBAL\ParameterType;

/**
 * SEUL endroit où vivent les fonctions spatiales propres à MariaDB. Les dépôts n'écrivent que
 * du SQL commun et appellent cette classe pour la boîte englobante et la distance.
 *
 * Équivalent PostGIS (colonne geom geography(Point,4326), index GIST) :
 *   - boîte englobante : `geom && ST_MakeEnvelope(:lonmin, :latmin, :lonmax, :latmax, 4326)`
 *     ou directement `ST_DWithin(geom, ST_MakePoint(:lon, :lat)::geography, :rayon_m)` (index utilisé) ;
 *   - distance en mètres : `ST_Distance(geom, ST_SetSRID(ST_MakePoint(:lon, :lat), 4326)::geography)`.
 * Pour porter : réécrire uniquement cette classe (et placer les paramètres nommés selon le pilote).
 */
final class Spatial
{
    private const METRES_PAR_DEGRE = 111_320.0;
    /**
     * Jusqu'à ce zoom (inclus), la requête « visibles dans la boîte » lit d'abord l'index sur min_zoom
     * (peu de lignes) ; au-delà, elle passe par l'index spatial (mesures : docs/osm.md).
     */
    public const SEUIL_ZOOM_INDEX_MIN_ZOOM = 9;

    /**
     * Source et condition (paramètres nommés) des points de $table visibles à $zoom dans la boîte
     * [ouest, sud, est, nord]. Les indications FORCE INDEX sont propres à MariaDB/MySQL : sous PostGIS,
     * les supprimer (le planificateur choisit) et écrire `geom && ST_MakeEnvelope(...)`.
     * $table est une constante des dépôts, jamais une entrée utilisateur.
     * @param array{0:float,1:float,2:float,3:float} $bbox
     * @return array{from:string,where:string,params:array<string,array{0:float|int|string,1:ParameterType}>} paramètres : nom => [valeur, type]
     */
    public function visibles(string $table, array $bbox, int $zoom): array
    {
        [$o, $s, $e, $n] = $bbox;
        if ($zoom <= self::SEUIL_ZOOM_INDEX_MIN_ZOOM) {
            return [
                'from' => "$table FORCE INDEX (ix_{$table}_min_zoom)",
                'where' => 'min_zoom <= :zoom AND lat BETWEEN :sud AND :nord AND lon BETWEEN :ouest AND :est',
                'params' => ['zoom' => [$zoom, ParameterType::INTEGER], 'sud' => [$s, ParameterType::STRING], 'nord' => [$n, ParameterType::STRING],
                    'ouest' => [$o, ParameterType::STRING], 'est' => [$e, ParameterType::STRING]],
            ];
        }
        return [
            'from' => "$table FORCE INDEX (sx_{$table}_geom)",
            'where' => $this->conditionBoite() . ' AND min_zoom <= :zoom',
            'params' => ['boite' => [$this->boiteDepuis($o, $s, $e, $n), ParameterType::STRING], 'zoom' => [$zoom, ParameterType::INTEGER]],
        ];
    }

    /** Boîte (ouest, sud, est, nord) en WKT, pour MBRContains. */
    public function boiteDepuis(float $ouest, float $sud, float $est, float $nord): string
    {
        return sprintf('POLYGON((%1$.7F %3$.7F,%2$.7F %3$.7F,%2$.7F %4$.7F,%1$.7F %4$.7F,%1$.7F %3$.7F))', $ouest, $est, $sud, $nord);
    }

    /** Condition SQL de pré-filtre sur l'index spatial ; paramètre nommé `:boite` = la boîte en WKT. */
    public function conditionBoite(string $colonne = 'geom'): string
    {
        return "MBRContains(ST_GeomFromText(:boite), $colonne)";
    }

    /** Boîte englobante (WKT, ordre lon lat) d'un cercle de rayon donné ; approximation sans traversée de l'antiméridien. */
    public function boiteWkt(float $lat, float $lon, float $rayonM): string
    {
        $dLat = $rayonM / self::METRES_PAR_DEGRE;
        $cos = max(cos(deg2rad($lat)), 0.01);
        $dLon = $rayonM / (self::METRES_PAR_DEGRE * $cos);
        [$x1, $x2] = [max($lon - $dLon, -180.0), min($lon + $dLon, 180.0)];
        [$y1, $y2] = [max($lat - $dLat, -90.0), min($lat + $dLat, 90.0)];
        return sprintf('POLYGON((%1$.7F %3$.7F,%2$.7F %3$.7F,%2$.7F %4$.7F,%1$.7F %4$.7F,%1$.7F %3$.7F))', $x1, $x2, $y1, $y2);
    }

    /** Expression SQL de la distance en mètres (sphère) ; paramètre nommé `:point` = le point en WKT. */
    public function distanceM(string $colonne = 'geom'): string
    {
        return "ST_Distance_Sphere($colonne, ST_GeomFromText(:point))";
    }

    public function pointWkt(float $lat, float $lon): string
    {
        return sprintf('POINT(%.7F %.7F)', $lon, $lat);
    }
}
