<?php
declare(strict_types=1);

namespace Carto\Repository;

use PDO;

final class LieuRepository
{
    public function __construct(private readonly PDO $pdo, private readonly Spatial $spatial = new Spatial()) {}

    /**
     * Lieux dont le nom commence par $prefixe (collation : casse et accents ignorés),
     * par population décroissante puis, si un point est fourni, par proximité.
     * @return list<array<string,mixed>>
     */
    public function parPrefixe(string $prefixe, int $limite, ?float $lat, ?float $lon, ?int $zoom = null): array
    {
        $like = addcslashes($prefixe, '\\%_') . '%';
        $params = [];
        $distSel = '';
        $order = 'COALESCE(population, 0) DESC';
        if ($lat !== null && $lon !== null) {
            $distSel = ', ' . $this->spatial->distanceM() . ' AS distance_m';
            $params[] = $this->spatial->pointWkt($lat, $lon);
            $order .= ', distance_m ASC';
        }
        $condZoom = $zoom !== null ? ' AND min_zoom <= ?' : '';
        $sql = "SELECT * FROM (SELECT osm_type, osm_id, categorie, nom, nom_alt, population, lat, lon, min_zoom$distSel
                FROM osm_lieux WHERE nom LIKE ?$condZoom) t ORDER BY $order, osm_id ASC LIMIT " . (int) $limite;
        $params[] = $like;
        if ($zoom !== null) {
            $params[] = $zoom;
        }
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }

    /**
     * Lieux visibles à $zoom dans la boîte : min_zoom croissant, puis population décroissante.
     * @param array{0:float,1:float,2:float,3:float} $bbox
     * @return list<array<string,mixed>>
     */
    public function visibles(array $bbox, int $zoom, int $limite): array
    {
        [$from, $where, $params] = $this->spatial->visibles('osm_lieux', $bbox, $zoom);
        $sql = "SELECT osm_type, osm_id, categorie, nom, nom_alt, population, lat, lon, min_zoom
                FROM $from WHERE $where ORDER BY min_zoom ASC, COALESCE(population, 0) DESC, osm_id ASC LIMIT " . (int) $limite;
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }
}
