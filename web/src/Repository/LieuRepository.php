<?php
declare(strict_types=1);

namespace Carto\Repository;

use Doctrine\DBAL\{Connection, ParameterType};

final class LieuRepository
{
    private const COLONNES = 'osm_type, osm_id, categorie, nom, nom_alt, population, lat, lon, min_zoom';

    public function __construct(private readonly Connection $db, private readonly Spatial $spatial = new Spatial()) {}

    /**
     * Lieux dont le nom commence par $prefixe (collation : casse et accents ignorés),
     * par population décroissante puis, si un point est fourni, par proximité.
     * @return list<array<string,mixed>>
     */
    public function parPrefixe(string $prefixe, int $limite, ?float $lat, ?float $lon, ?int $zoom = null): array
    {
        $interne = $this->db->createQueryBuilder()
            ->select(self::COLONNES)->from('osm_lieux')
            ->where('nom LIKE :motif')
            ->setParameter('motif', addcslashes($prefixe, '\\%_') . '%');
        $tri = [['COALESCE(population, 0)', 'DESC']];
        if ($lat !== null && $lon !== null) {
            $interne->addSelect($this->spatial->distanceM() . ' AS distance_m')->setParameter('point', $this->spatial->pointWkt($lat, $lon));
            $tri[] = ['distance_m', 'ASC'];
        }
        if ($zoom !== null) {
            $interne->andWhere('min_zoom <= :zoom')->setParameter('zoom', $zoom, ParameterType::INTEGER);
        }
        $tri[] = ['osm_id', 'ASC'];
        $qb = $this->db->createQueryBuilder()
            ->select('*')->from('(' . $interne->getSQL() . ')', 't')
            ->setMaxResults($limite)
            ->setParameters($interne->getParameters(), $interne->getParameterTypes());
        foreach ($tri as [$expr, $sens]) {
            $qb->addOrderBy($expr, $sens);
        }
        return $qb->executeQuery()->fetchAllAssociative();
    }

    /**
     * Lieux visibles à $zoom dans la boîte : min_zoom croissant, puis population décroissante.
     * @param array{0:float,1:float,2:float,3:float} $bbox
     * @return list<array<string,mixed>>
     */
    public function visibles(array $bbox, int $zoom, int $limite): array
    {
        $v = $this->spatial->visibles('osm_lieux', $bbox, $zoom);
        $qb = $this->db->createQueryBuilder()
            ->select(self::COLONNES)->from($v['from'])->where($v['where'])
            ->orderBy('min_zoom', 'ASC')->addOrderBy('COALESCE(population, 0)', 'DESC')->addOrderBy('osm_id', 'ASC')
            ->setMaxResults($limite);
        foreach ($v['params'] as $nom => [$valeur, $type]) {
            $qb->setParameter($nom, $valeur, $type);
        }
        return $qb->executeQuery()->fetchAllAssociative();
    }
}
