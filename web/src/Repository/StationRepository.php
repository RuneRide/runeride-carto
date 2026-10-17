<?php
declare(strict_types=1);

namespace Carto\Repository;

use PDO;

final class StationRepository
{
    public function __construct(private readonly PDO $pdo, private readonly Spatial $spatial = new Spatial()) {}

    /**
     * Stations dans le rayon, triées par distance croissante.
     * @return list<array<string,mixed>>
     */
    public function autour(float $lat, float $lon, float $rayonM, ?string $marque, int $limite): array
    {
        $dist = $this->spatial->distanceM();
        $sql = "SELECT osm_type, osm_id, nom, marque, exploitant, carburants, adr_numero, adr_rue, adr_cp, adr_ville, lat, lon, $dist AS distance_m
                FROM osm_stations
                WHERE {$this->spatial->conditionBoite()}";
        $params = [$this->spatial->pointWkt($lat, $lon), $this->spatial->boiteWkt($lat, $lon, $rayonM)];
        if ($marque !== null) {
            $sql .= ' AND marque = ?'; // comparaison insensible à la casse et aux accents via la collation
            $params[] = $marque;
        }
        // Le filtre exact (cercle) est appliqué par HAVING-équivalent : sous-requête pour rester en SQL commun.
        $sql = "SELECT * FROM ($sql) t WHERE distance_m <= ? ORDER BY distance_m ASC, osm_id ASC LIMIT " . (int) $limite;
        $params[] = $rayonM;
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }
}
