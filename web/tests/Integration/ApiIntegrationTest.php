<?php
declare(strict_types=1);

namespace Carto\Tests\Integration;

use Carto\{Api, Config, Database, Router};
use Doctrine\DBAL\Connection;
use PDO;
use PHPUnit\Framework\TestCase;

/**
 * Demande la base OSM chargée (CARTO_TEST_DSN, CARTO_TEST_USER, CARTO_TEST_PASSWORD) ; sinon ignoré.
 * Compte en lecture seule suffisant. S'appuie sur l'extrait France de docs/osm.md (Toulouse, Paris...).
 */
final class ApiIntegrationTest extends TestCase
{
    /** @var array<string,mixed> */
    private array $query = [];
    private Connection $conn;
    private PDO $pdo; // même connexion, API PDO native pour les vérifications directes

    protected function setUp(): void
    {
        $dsn = getenv('CARTO_TEST_DSN');
        if (!$dsn) {
            $this->markTestSkipped('CARTO_TEST_DSN absent');
        }
        $this->conn = Database::connect(new Config([
            'CARTO_DB_DSN' => $dsn, 'CARTO_DB_USER' => (string) getenv('CARTO_TEST_USER'), 'CARTO_DB_PASSWORD' => (string) getenv('CARTO_TEST_PASSWORD'),
        ]));
        $this->pdo = $this->conn->getNativeConnection();
    }

    /** @param array<string,string> $q @return array{0:int,1:array<string,mixed>} */
    private function get(string $path, array $q = []): array
    {
        $this->query = $q;
        // $this->query est passé par référence à Api::register : on réutilise le même tableau
        $router = new Router();
        Api::register($router, fn () => $this->conn, 'test', $this->query);
        return $router->dispatch('GET', $path);
    }

    public function testSante(): void
    {
        [$code, $body] = $this->get('/v1/sante');
        $this->assertSame(200, $code);
        $this->assertSame('ok', $body['statut']);
    }

    public function testStationsAutourDeToulouse(): void
    {
        [$code, $body] = $this->get('/v1/stations', ['lat' => '43.6045', 'lon' => '1.4442', 'rayon' => '10']);
        $this->assertSame(200, $code);
        $this->assertNotEmpty($body['donnees']);
        $this->assertLessThanOrEqual(50, count($body['donnees']));
        $prev = -1;
        foreach ($body['donnees'] as $s) {
            $this->assertLessThanOrEqual(10000, $s['distance_m']);
            $this->assertGreaterThanOrEqual($prev, $s['distance_m'], 'trié par distance');
            $prev = $s['distance_m'];
            $this->assertIsArray($s['carburants']);
        }
        $this->assertStringContainsString('OpenStreetMap', $body['attribution']);
    }

    public function testStationsRayonPlusGrandDonnePlus(): void
    {
        $q = ['lat' => '43.6045', 'lon' => '1.4442', 'limite' => '100'];
        $petit = $this->get('/v1/stations', $q + ['rayon' => '2'])[1]['donnees'];
        $grand = $this->get('/v1/stations', $q + ['rayon' => '30'])[1]['donnees'];
        $this->assertGreaterThan(count($petit), count($grand));
        $this->assertLessThanOrEqual(100, count($grand));
    }

    public function testStationsFiltreMarqueEtLimite(): void
    {
        $tous = $this->get('/v1/stations', ['lat' => '43.6045', 'lon' => '1.4442', 'rayon' => '30', 'limite' => '100'])[1]['donnees'];
        $marque = null;
        foreach ($tous as $s) {
            if ($s['marque'] !== null) { $marque = $s['marque']; break; }
        }
        $this->assertNotNull($marque);
        $f = $this->get('/v1/stations', ['lat' => '43.6045', 'lon' => '1.4442', 'rayon' => '30', 'marque' => strtoupper($marque), 'limite' => '3'])[1]['donnees'];
        $this->assertNotEmpty($f);
        $this->assertLessThanOrEqual(3, count($f));
        foreach ($f as $s) {
            $this->assertSame(mb_strtolower($marque), mb_strtolower($s['marque']));
        }
    }

    public function testStationsEnMerVide(): void
    {
        [$code, $body] = $this->get('/v1/stations', ['lat' => '0', 'lon' => '-30']);
        $this->assertSame(200, $code);
        $this->assertSame([], $body['donnees']);
    }

    public function testStationsValidation(): void
    {
        $this->assertSame(400, $this->get('/v1/stations', ['lat' => '95', 'lon' => '1'])[0]);
        $this->assertSame(400, $this->get('/v1/stations', ['lat' => '43', 'lon' => '1', 'rayon' => '51'])[0]);
        $this->assertSame(400, $this->get('/v1/stations', ['lat' => '43', 'lon' => '1', 'limite' => '101'])[0]);
        $this->assertSame(400, $this->get('/v1/stations', ['lon' => '1'])[0]);
    }

    public function testLieuxPrefixeCasseEtAccents(): void
    {
        $a = $this->get('/v1/lieux', ['q' => 'Toulou'])[1]['donnees'];
        $b = $this->get('/v1/lieux', ['q' => 'toulou'])[1]['donnees'];
        $this->assertNotEmpty($a);
        $this->assertSame('Toulouse', $a[0]['nom']);
        $this->assertSame($a, $b);
        $c = $this->get('/v1/lieux', ['q' => 'Chateau', 'limite' => '100'])[1]['donnees'];
        $d = $this->get('/v1/lieux', ['q' => 'Château', 'limite' => '100'])[1]['donnees'];
        $this->assertSame($c, $d, 'accents ignorés');
    }

    public function testLieuxTriParPopulationPuisProximite(): void
    {
        $r = $this->get('/v1/lieux', ['q' => 'Saint-Ma', 'limite' => '100'])[1]['donnees'];
        $pop = array_map(fn ($l) => $l['population'] ?? 0, $r);
        $triees = $pop;
        rsort($triees);
        $this->assertSame($triees, $pop);

        // Sans population (hameaux homonymes), la proximité départage
        $r = $this->get('/v1/lieux', ['q' => 'Le Bois', 'lat' => '43.6', 'lon' => '1.44', 'limite' => '100'])[1]['donnees'];
        $this->assertNotEmpty($r);
        $groupes = [];
        foreach ($r as $l) {
            $groupes[$l['population'] ?? 0][] = $l['distance_m'];
        }
        foreach ($groupes as $distances) {
            $tri = $distances;
            sort($tri);
            $this->assertSame($tri, $distances);
        }
    }

    public function testLieuxJokersLittérauxEtValidation(): void
    {
        $this->assertSame([], $this->get('/v1/lieux', ['q' => '%%'])[1]['donnees'], '% n\'est pas un joker');
        $this->assertSame(400, $this->get('/v1/lieux', [])[0]);
        $this->assertSame(400, $this->get('/v1/lieux', ['q' => 'a'])[0]);
        $this->assertSame(400, $this->get('/v1/lieux', ['q' => 'Toulouse', 'lat' => '43'])[0]);
        $this->assertSame(400, $this->get('/v1/lieux', ['q' => 'Toulouse', 'limite' => '500'])[0]);
    }

    // ---- min_zoom, zoom, /v1/vue (lot 2) ----

    public function testZoomFiltreStationsEtLieux(): void
    {
        $q = ['lat' => '43.6045', 'lon' => '1.4442', 'rayon' => '12', 'limite' => '100'];
        $tout = $this->get('/v1/stations', $q)[1]['donnees'];
        $z11 = $this->get('/v1/stations', $q + ['zoom' => '11'])[1]['donnees'];
        $z15 = $this->get('/v1/stations', $q + ['zoom' => '15'])[1]['donnees'];
        $this->assertEquals($tout, $z15, 'à z15 tout est visible');
        $this->assertLessThan(count($tout), count($z11));
        $this->assertNotEmpty($z11);
        foreach ($z11 as $s) {
            $this->assertLessThanOrEqual(11, $s['min_zoom']);
        }
        $this->assertSame([], $this->get('/v1/stations', $q + ['zoom' => '0'])[1]['donnees']);

        $ids = fn (array $r) => array_column($r, 'id');
        $z4 = $this->get('/v1/lieux', ['q' => 'Toulouse', 'zoom' => '4', 'limite' => '100'])[1]['donnees'];
        $z5 = $this->get('/v1/lieux', ['q' => 'Toulouse', 'zoom' => '5', 'limite' => '100'])[1]['donnees'];
        $tousL = $this->get('/v1/lieux', ['q' => 'Toulouse', 'limite' => '100'])[1]['donnees'];
        $this->assertNotContains('n/26686518', $ids($z4), 'Toulouse (min_zoom 5) invisible à z4');
        $this->assertContains('n/26686518', $ids($z5), 'visible à z5');
        $this->assertContains('n/26686518', $ids($tousL), 'et sans zoom');
        foreach ($z4 as $x) {
            $this->assertLessThanOrEqual(4, $x['min_zoom']);
        }
    }

    public function testVueCoherenteAvecMinZoomAuxDeuxStrategies(): void
    {
        // z9 (index min_zoom) et z10 (index spatial) : mêmes règles, résultats vérifiés contre un calcul direct
        foreach ([[9, '0.8', '0.5'], [10, '0.5', '0.35'], [4, '12', '8'], [15, '0.06', '0.04']] as [$z, $w, $h]) {
            $o = 1.44 - (float) $w / 2; $e = 1.44 + (float) $w / 2; $s = 43.6 - (float) $h / 2; $n = 43.6 + (float) $h / 2;
            [$code, $body] = $this->get('/v1/vue', ['zoom' => (string) $z, 'bbox' => "$o,$s,$e,$n", 'limite' => '500']);
            $this->assertSame(200, $code, "z$z");
            $this->assertFalse($body['meta']['tronque']);
            $attendu = 0;
            foreach (['osm_lieux', 'osm_stations'] as $t) {
                $st = $this->pdo->prepare("SELECT COUNT(*) FROM $t WHERE min_zoom <= ? AND lat BETWEEN ? AND ? AND lon BETWEEN ? AND ?");
                $st->execute([$z, $s, $n, $o, $e]);
                $attendu += (int) $st->fetchColumn();
            }
            $this->assertSame($attendu, $body['meta']['nombre'], "z$z : nombre de points visibles");
            foreach ($body['donnees']['lieux'] as $x) {
                $this->assertLessThanOrEqual($z, $x['min_zoom']);
                $this->assertGreaterThanOrEqual($s, $x['lat']);
                $this->assertLessThanOrEqual($n, $x['lat']);
                $this->assertGreaterThanOrEqual($o, $x['lon']);
                $this->assertLessThanOrEqual($e, $x['lon']);
                $this->assertArrayNotHasKey('distance_m', $x);
            }
            $this->assertStringContainsString('OpenStreetMap', $body['attribution']);
        }
    }

    public function testVueLimiteTotaleEtTroncature(): void
    {
        [$code, $body] = $this->get('/v1/vue', ['zoom' => '15', 'bbox' => '2.30,48.84,2.38,48.88', 'limite' => '10']);
        $this->assertSame(200, $code);
        $this->assertSame(10, $body['meta']['nombre']);
        $this->assertTrue($body['meta']['tronque']);
        $this->assertSame(10, count($body['donnees']['stations']) + count($body['donnees']['lieux']));
        // les plus importants d'abord : le plus grand min_zoom gardé n'est pas inférieur à ceux écartés ; ici, on vérifie le tri
        $zooms = array_merge(array_column($body['donnees']['stations'], 'min_zoom'), array_column($body['donnees']['lieux'], 'min_zoom'));
        $this->assertLessThanOrEqual(15, max($zooms));
        $grand = $this->get('/v1/vue', ['zoom' => '15', 'bbox' => '2.30,48.84,2.38,48.88', 'limite' => '500'])[1];
        $this->assertGreaterThan(10, $grand['meta']['nombre']);
        $this->assertLessThanOrEqual(500, $grand['meta']['nombre']);
        $garde = $zooms;
        sort($garde);
        $tous = array_merge(array_column($grand['donnees']['stations'], 'min_zoom'), array_column($grand['donnees']['lieux'], 'min_zoom'));
        sort($tous);
        $this->assertSame($garde, array_slice($tous, 0, 10), 'la troncature garde les points de plus petit min_zoom');
    }

    public function testVueValidation(): void
    {
        $this->assertSame(400, $this->get('/v1/vue', ['bbox' => '1,43,1.1,43.1'])[0], 'zoom requis');
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10'])[0], 'bbox requise');
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10', 'bbox' => '1,43,5,47'])[0], 'trop grande à z10');
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '23', 'bbox' => '1,43,1.01,43.01'])[0]);
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10', 'bbox' => '1,43,1.1,43.1', 'limite' => '501'])[0]);
        $this->assertSame(400, $this->get('/v1/stations', ['lat' => '43', 'lon' => '1', 'zoom' => '99'])[0]);
        $this->assertSame(400, $this->get('/v1/lieux', ['q' => 'Toulouse', 'zoom' => '-1'])[0]);
    }

    /** @return iterable<string,array{0:string}> */
    public static function charges(): iterable
    {
        yield 'quote' => ["5' OR '1'='1"];
        yield 'drop' => ["5; DROP TABLE osm_lieux;--"];
        yield 'union' => ["5 UNION SELECT 1,2,3"];
        yield 'commentaire' => ["5/**/OR/**/1=1"];
        yield 'nul' => ["5\0"];
        yield 'hex' => ["0x10"];
        yield 'espace' => ["5 "];
        yield 'retour ligne' => ["5\n"];
        yield 'sleep' => ["(SELECT SLEEP(5))"];
    }

    #[\PHPUnit\Framework\Attributes\DataProvider('charges')]
    public function testInjectionDansLesNouveauxParametres(string $charge): void
    {
        $avant = (int) $this->pdo->query('SELECT COUNT(*) FROM osm_lieux')->fetchColumn();
        $t0 = microtime(true);
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => $charge, 'bbox' => '1,43,1.1,43.1'])[0]);
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10', 'bbox' => '1,43,1.1,' . $charge])[0]);
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10', 'bbox' => $charge . ',43,1.1,43.1'])[0]);
        $this->assertSame(400, $this->get('/v1/vue', ['zoom' => '10', 'bbox' => '1,43,1.1,43.1', 'limite' => $charge])[0]);
        $this->assertSame(400, $this->get('/v1/stations', ['lat' => '43', 'lon' => '1', 'zoom' => $charge])[0]);
        $this->assertSame(400, $this->get('/v1/lieux', ['q' => 'Toulouse', 'zoom' => $charge])[0]);
        $this->assertLessThan(2.0, microtime(true) - $t0, 'aucun SLEEP exécuté');
        $this->assertSame($avant, (int) $this->pdo->query('SELECT COUNT(*) FROM osm_lieux')->fetchColumn());
    }

    public function testChargeEnTableauNeCasseRien(): void
    {
        $this->query = ['zoom' => ['1'], 'bbox' => ['1,43,1.1,43.1']];
        $this->assertSame(400, $this->get('/v1/vue', $this->query)[0]);
    }
}
