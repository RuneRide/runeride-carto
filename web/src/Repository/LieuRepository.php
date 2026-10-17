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
    public function parPrefixe(string $prefixe, int $limite, ?float $lat, ?float $lon): array
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
        $sql = "SELECT * FROM (SELECT osm_type, osm_id, categorie, nom, nom_alt, population, lat, lon$distSel
                FROM osm_lieux WHERE nom LIKE ?) t ORDER BY $order, osm_id ASC LIMIT " . (int) $limite;
        $params[] = $like;
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }
}
