<?php
declare(strict_types=1);

namespace Carto\Repository;

use Doctrine\DBAL\{Connection, ParameterType};

final class StationRepository
{
    private const COLONNES = 'osm_type, osm_id, nom, marque, exploitant, carburants, adr_numero, adr_rue, adr_cp, adr_ville, lat, lon, min_zoom';

    public function __construct(private readonly Connection $db, private readonly Spatial $spatial = new Spatial()) {}

    /**
     * Stations dans le rayon, triées par distance croissante.
     * @return list<array<string,mixed>>
     */
    public function autour(float $lat, float $lon, float $rayonM, ?string $marque, int $limite, ?int $zoom = null): array
    {
        $interne = $this->db->createQueryBuilder()
            ->select(self::COLONNES, $this->spatial->distanceM() . ' AS distance_m')
            ->from('osm_stations')
            ->where($this->spatial->conditionBoite())
            ->setParameter('point', $this->spatial->pointWkt($lat, $lon))
            ->setParameter('boite', $this->spatial->boiteWkt($lat, $lon, $rayonM));
        if ($marque !== null) {
            $interne->andWhere('marque = :marque')->setParameter('marque', $marque); // insensible à la casse et aux accents via la collation
        }
        if ($zoom !== null) {
            $interne->andWhere('min_zoom <= :zoom')->setParameter('zoom', $zoom, ParameterType::INTEGER);
        }
        // Filtre exact (cercle) sur une sous-requête : reste du SQL commun, sans HAVING.
        return $this->db->createQueryBuilder()
            ->select('*')
            ->from('(' . $interne->getSQL() . ')', 't')
            ->where('distance_m <= :rayon')
            ->orderBy('distance_m', 'ASC')->addOrderBy('osm_id', 'ASC')
            ->setMaxResults($limite)
            ->setParameters($interne->getParameters() + ['rayon' => $rayonM], $interne->getParameterTypes())
            ->executeQuery()->fetchAllAssociative();
    }

    /**
     * Stations visibles à $zoom dans la boîte, les plus importantes d'abord (min_zoom croissant).
     * Renvoie au plus $limite lignes (l'appelant demande limite + 1 pour savoir s'il y a troncature).
     * @param array{0:float,1:float,2:float,3:float} $bbox
     * @return list<array<string,mixed>>
     */
    public function visibles(array $bbox, int $zoom, int $limite): array
    {
        $v = $this->spatial->visibles('osm_stations', $bbox, $zoom);
        $qb = $this->db->createQueryBuilder()
            ->select(self::COLONNES)->from($v['from'])->where($v['where'])
            ->orderBy('min_zoom', 'ASC')->addOrderBy('osm_id', 'ASC')
            ->setMaxResults($limite);
        foreach ($v['params'] as $nom => [$valeur, $type]) {
            $qb->setParameter($nom, $valeur, $type);
        }
        return $qb->executeQuery()->fetchAllAssociative();
    }
}
