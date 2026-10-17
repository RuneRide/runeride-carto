#!/usr/bin/env php
<?php
declare(strict_types=1);

/**
 * Comparaison « points visibles dans une boîte à zoom z » (voir docs/osm.md).
 *   (a) index spatial seul + filtre min_zoom        (FORCE INDEX sx_*_geom)
 *   (d) index sur min_zoom + filtre lat/lon         (FORCE INDEX ix_*_min_zoom)
 * Si les tables ont en plus une colonne entière `cell` (clé de tuile Web Mercator au niveau 12, x*4096+y)
 * et les index ix_*_cell (cell, min_zoom) / ix_*_zoom_cell (min_zoom, cell), on mesure aussi :
 *   (b) clé de cellule + min_zoom, (c) idem avec min_zoom en tête de l'index.
 * Ces colonnes ne sont PAS dans le schéma livré (la mesure ne les a pas justifiées) : voir docs/osm.md pour les recréer.
 * Usage : CARTO_BENCH_DSN=... CARTO_BENCH_USER=... CARTO_BENCH_PASSWORD=... php bin/bench-zoom.php [repetitions]
 */
require __DIR__ . '/../web/vendor/autoload.php';

use Carto\Repository\Spatial;

const NIVEAU = 12;

/** @return array{0:int,1:int} tuile Web Mercator (identique à tuile() de bin/carto_osm.py) */
function tuile(float $lat, float $lon, int $z): array
{
    $n = 1 << $z;
    $lat = max(-85.0511287798, min(85.0511287798, $lat));
    $x = (int) floor(($lon + 180.0) / 360.0 * $n);
    $y = (int) floor((1.0 - asinh(tan(deg2rad($lat))) / M_PI) / 2.0 * $n);
    return [min(max($x, 0), $n - 1), min(max($y, 0), $n - 1)];
}

$reps = (int) ($argv[1] ?? 30);
$pdo = new PDO(getenv('CARTO_BENCH_DSN') ?: 'mysql:host=127.0.0.1;port=3307;dbname=runeride_osm;charset=utf8mb4',
    getenv('CARTO_BENCH_USER') ?: 'carto_ro', getenv('CARTO_BENCH_PASSWORD') ?: 'carto_ro',
    [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION, PDO::ATTR_EMULATE_PREPARES => false, PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC]);
$sp = new Spatial();

// centres : dense (Paris), ville moyenne (Toulouse), rural (Lozère), côte (Bretagne), Alpes
$centres = [[48.86, 2.35], [43.60, 1.44], [44.50, 3.50], [48.10, -2.80], [45.20, 6.30]];
// (zoom, largeur°, hauteur°) : une boîte « écran » et une boîte « grand écran » par zoom
$cas = [[4, 12.0, 8.0], [4, 30.0, 20.0], [8, 1.5, 1.0], [8, 4.0, 3.0], [9, 0.8, 0.5], [9, 2.0, 1.4], [10, 0.5, 0.35], [10, 1.2, 0.8], [11, 0.25, 0.17], [11, 0.6, 0.4], [12, 0.10, 0.07], [12, 0.30, 0.20], [15, 0.02, 0.012], [15, 0.06, 0.04]];

function variantes(Spatial $sp, string $t, array $b, int $z, bool $avecCell): array
{
    [$o, $s, $e, $n] = $b;
    $cols = 'osm_type, osm_id, nom, lat, lon, min_zoom';
    $v = ['a' => ["SELECT $cols FROM $t FORCE INDEX (sx_{$t}_geom) WHERE MBRContains(ST_GeomFromText(?), geom) AND min_zoom <= ?",
        [$sp->boiteDepuis($o, $s, $e, $n), $z]]];
    $v['d'] = ["SELECT $cols FROM $t FORCE INDEX (ix_{$t}_min_zoom) WHERE min_zoom <= ? AND lat BETWEEN ? AND ? AND lon BETWEEN ? AND ?", [$z, $s, $n, $o, $e]];
    if ($avecCell) {
        [$x1, $y1] = tuile($n, $o, NIVEAU);
        [$x2, $y2] = tuile($s, $e, NIVEAU);
        $plages = [];
        for ($x = $x1; $x <= $x2; $x++) {
            $plages[] = [($x << NIVEAU) | $y1, ($x << NIVEAU) | $y2];
        }
        $ou = implode(' OR ', array_fill(0, count($plages), '(cell BETWEEN ? AND ?)'));
        $filtre = " WHERE ($ou) AND min_zoom <= ? AND lat BETWEEN ? AND ? AND lon BETWEEN ? AND ?";
        $params = array_merge(array_merge(...$plages), [$z, $s, $n, $o, $e]);
        $v['b'] = ["SELECT $cols FROM $t FORCE INDEX (ix_{$t}_cell)$filtre", $params];
        $v['c'] = ["SELECT $cols FROM $t FORCE INDEX (ix_{$t}_zoom_cell)$filtre", $params];
    }
    return $v;
}

function mediane(array $a): float { sort($a); $n = count($a); return $n % 2 ? $a[intdiv($n, 2)] : ($a[$n / 2 - 1] + $a[$n / 2]) / 2; }

printf("repetitions=%d (+1 d'echauffement ignore par requete), medianes sur les centres ; temps en ms\n", $reps);
foreach (['osm_lieux', 'osm_stations'] as $t) {
    $avecC = (bool) $pdo->query("SELECT COUNT(*) FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = '$t' AND column_name = 'cell'")->fetchColumn();
    echo "\n== $t ==\n";
    printf("%-3s %-13s %-3s %9s %10s %10s %7s %s\n", 'z', 'boite deg', 'var', 'resultats', 'lignes_ex', 't_med_ms', 't_max', 'plan');
    foreach ($cas as [$z, $w, $h]) {
        $agg = [];
        foreach ($centres as [$la, $lo]) {
            $b = [$lo - $w / 2, $la - $h / 2, $lo + $w / 2, $la + $h / 2];
            foreach (variantes($sp, $t, $b, $z, $avecC) as $nom => [$sql, $params]) {
                $st = $pdo->prepare($sql);
                $st->execute($params);
                $nb = count($st->fetchAll()); // échauffement (caches chauds)
                $tm = [];
                for ($i = 0; $i < $reps; $i++) {
                    $t0 = hrtime(true);
                    $st->execute($params);
                    $st->fetchAll();
                    $tm[] = (hrtime(true) - $t0) / 1e6;
                }
                // lignes réellement examinées : ANALYZE (r_rows) ; plan : EXPLAIN (type/key)
                $an = $pdo->prepare('ANALYZE ' . $sql);
                $an->execute($params);
                $r = $an->fetch();
                $ex = $pdo->prepare('EXPLAIN ' . $sql);
                $ex->execute($params);
                $e = $ex->fetch();
                $agg[$nom][] = ['nb' => $nb, 'rows' => (float) ($r['r_rows'] ?? -1), 'tm' => mediane($tm), 'max' => max($tm), 'plan' => ($e['type'] ?? '?') . '/' . ($e['key'] ?? 'NULL')];
            }
        }
        foreach ($agg as $nom => $l) {
            printf("%-3d %-13s %-3s %9d %10d %10.3f %7.3f %s\n", $z, "{$w}x{$h}", $nom,
                (int) mediane(array_column($l, 'nb')), (int) mediane(array_column($l, 'rows')),
                mediane(array_column($l, 'tm')), max(array_column($l, 'max')), $l[0]['plan']);
        }
    }
}
