<?php
declare(strict_types=1);

namespace Carto\Tests\Integration;

use Carto\{Api, Config, Database, Router};
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
    private PDO $pdo;

    protected function setUp(): void
    {
        $dsn = getenv('CARTO_TEST_DSN');
        if (!$dsn) {
            $this->markTestSkipped('CARTO_TEST_DSN absent');
        }
        $this->pdo = Database::connect(new Config([
            'CARTO_DB_DSN' => $dsn, 'CARTO_DB_USER' => (string) getenv('CARTO_TEST_USER'), 'CARTO_DB_PASSWORD' => (string) getenv('CARTO_TEST_PASSWORD'),
        ]));
    }

    /** @param array<string,string> $q @return array{0:int,1:array<string,mixed>} */
    private function get(string $path, array $q = []): array
    {
        $this->query = $q;
        // $this->query est passé par référence à Api::register : on réutilise le même tableau
        $router = new Router();
        Api::register($router, fn () => $this->pdo, 'test', $this->query);
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
}
