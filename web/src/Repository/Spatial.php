<?php
declare(strict_types=1);

namespace Carto\Repository;

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

    /** Condition SQL de pré-filtre sur l'index spatial ; un paramètre : la boîte en WKT. */
    public function conditionBoite(string $colonne = 'geom'): string
    {
        return "MBRContains(ST_GeomFromText(?), $colonne)";
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

    /** Expression SQL de la distance en mètres (sphère) ; un paramètre : le point en WKT. */
    public function distanceM(string $colonne = 'geom'): string
    {
        return "ST_Distance_Sphere($colonne, ST_GeomFromText(?))";
    }

    public function pointWkt(float $lat, float $lon): string
    {
        return sprintf('POINT(%.7F %.7F)', $lon, $lat);
    }
}
