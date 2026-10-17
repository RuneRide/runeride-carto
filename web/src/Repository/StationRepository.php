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
    public function autour(float $lat, float $lon, float $rayonM, ?string $marque, int $limite, ?int $zoom = null): array
    {
        $dist = $this->spatial->distanceM();
        $sql = "SELECT osm_type, osm_id, nom, marque, exploitant, carburants, adr_numero, adr_rue, adr_cp, adr_ville, lat, lon, min_zoom, $dist AS distance_m
                FROM osm_stations
                WHERE {$this->spatial->conditionBoite()}";
        $params = [$this->spatial->pointWkt($lat, $lon), $this->spatial->boiteWkt($lat, $lon, $rayonM)];
        if ($marque !== null) {
            $sql .= ' AND marque = ?'; // comparaison insensible à la casse et aux accents via la collation
            $params[] = $marque;
        }
        if ($zoom !== null) {
            $sql .= ' AND min_zoom <= ?';
            $params[] = $zoom;
        }
        // Le filtre exact (cercle) est appliqué par HAVING-équivalent : sous-requête pour rester en SQL commun.
        $sql = "SELECT * FROM ($sql) t WHERE distance_m <= ? ORDER BY distance_m ASC, osm_id ASC LIMIT " . (int) $limite;
        $params[] = $rayonM;
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }

    /**
     * Stations visibles à $zoom dans la boîte, les plus importantes d'abord (min_zoom croissant).
     * Renvoie au plus $limite lignes (l'appelant demande limite + 1 pour savoir s'il y a troncature).
     * @param array{0:float,1:float,2:float,3:float} $bbox
     * @return list<array<string,mixed>>
     */
    public function visibles(array $bbox, int $zoom, int $limite): array
    {
        [$from, $where, $params] = $this->spatial->visibles('osm_stations', $bbox, $zoom);
        $sql = "SELECT osm_type, osm_id, nom, marque, exploitant, carburants, adr_numero, adr_rue, adr_cp, adr_ville, lat, lon, min_zoom
                FROM $from WHERE $where ORDER BY min_zoom ASC, osm_id ASC LIMIT " . (int) $limite;
        $st = $this->pdo->prepare($sql);
        $st->execute($params);
        return $st->fetchAll();
    }
}
