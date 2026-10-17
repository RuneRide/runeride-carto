<?php
declare(strict_types=1);

namespace Carto\Tests\Integration;

use Carto\{Api, Config, Database, Router};
use Doctrine\DBAL\Connection;
use PDO;
use PHPUnit\Framework\TestCase;

/**
 * Import de bout en bout avec des noms OSM hostiles : bin/carto_osm.py (TSV + charger.sql), puis
 * `mariadb --local-infile=1` comme sur l'hébergeur, dans une base jetable créée puis supprimée.
 *
 * Demande un compte administrateur sur un MariaDB de test (jamais la base de production) :
 * CARTO_TEST_ADMIN_HOST, CARTO_TEST_ADMIN_PORT, CARTO_TEST_ADMIN_USER, CARTO_TEST_ADMIN_PASSWORD,
 * et les commandes `python3` et `mariadb` dans le PATH ; sinon ignoré.
 */
final class ImportSurTest extends TestCase
{
    private const BASE = 'carto_test_import';
    private static string $dossier = '';
    private static ?Connection $conn = null;
    private static ?PDO $pdo = null; // même connexion, API PDO native pour les vérifications directes
    /** @var array<string,array<string,array<string,mixed>>> */
    private static array $attendu = [];

    public static function setUpBeforeClass(): void
    {
        $host = getenv('CARTO_TEST_ADMIN_HOST');
        if (!$host) {
            self::markTestSkipped('CARTO_TEST_ADMIN_HOST absent');
        }
        $port = (string) getenv('CARTO_TEST_ADMIN_PORT') ?: '3306';
        $user = (string) getenv('CARTO_TEST_ADMIN_USER');
        $pass = (string) getenv('CARTO_TEST_ADMIN_PASSWORD');
        $racine = dirname(__DIR__, 3);

        self::$dossier = sys_get_temp_dir() . '/carto-import-' . bin2hex(random_bytes(4));
        mkdir(self::$dossier, 0700, true);
        self::sh(['python3', $racine . '/bin/carto_osm.py', 'pieges', self::$dossier], self::$dossier);
        self::$attendu = json_decode((string) file_get_contents(self::$dossier . '/attendu.json'), true, 512, JSON_THROW_ON_ERROR);

        $admin = new PDO("mysql:host=$host;port=$port;charset=utf8mb4", $user, $pass, [PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION]);
        $admin->exec('DROP DATABASE IF EXISTS ' . self::BASE);
        $admin->exec('CREATE DATABASE ' . self::BASE . ' CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci');

        $cli = ['mariadb', '--local-infile=1', "--host=$host", "--port=$port", "--user=$user", '--protocol=tcp', self::BASE];
        $env = ['MYSQL_PWD' => $pass];
        self::sh($cli, $racine, $env, $racine . '/docs/osm/osm_schema.sql');
        // Table sentinelle : « '); DROP TABLE x;-- » ne doit pas la supprimer.
        self::sh($cli, $racine, $env, null, 'CREATE TABLE x (a INT); INSERT INTO x VALUES (42);');
        $sortie = self::sh($cli, self::$dossier, $env, self::$dossier . '/charger.sql');
        self::assertSame('', trim($sortie), 'SHOW WARNINGS doit être vide : ' . $sortie);

        self::$conn = Database::connect(new Config([
            'CARTO_DB_DSN' => "mysql:host=$host;port=$port;dbname=" . self::BASE . ';charset=utf8mb4',
            'CARTO_DB_USER' => $user, 'CARTO_DB_PASSWORD' => $pass,
        ]));
        self::$pdo = self::$conn->getNativeConnection();
    }

    public static function tearDownAfterClass(): void
    {
        if (self::$pdo !== null) {
            self::$pdo->exec('DROP DATABASE IF EXISTS ' . self::BASE);
            self::$pdo = null;
            self::$conn = null;
        }
        if (self::$dossier !== '' && is_dir(self::$dossier)) {
            foreach (glob(self::$dossier . '/*') ?: [] as $f) {
                unlink($f);
            }
            rmdir(self::$dossier);
        }
    }

    /** @param list<string> $cmd @param array<string,string> $env */
    private static function sh(array $cmd, string $cwd, array $env = [], ?string $stdinFile = null, ?string $stdin = null): string
    {
        $p = proc_open($cmd, [0 => ['pipe', 'r'], 1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes, $cwd, $env + ['PATH' => (string) getenv('PATH')]);
        fwrite($pipes[0], $stdinFile !== null ? (string) file_get_contents($stdinFile) : (string) $stdin);
        fclose($pipes[0]);
        $out = (string) stream_get_contents($pipes[1]);
        $err = (string) stream_get_contents($pipes[2]);
        $code = proc_close($p);
        self::assertSame(0, $code, implode(' ', $cmd) . " a échoué : $err");
        return $out;
    }

    public function testToutesLesLignesSontLuesTellesQuellesParTable(): void
    {
        foreach (['osm_lieux' => 'categorie', 'osm_stations' => 'adr_rue'] as $table => $_) {
            $rows = self::$pdo->query("SELECT *, ST_X(geom) AS gx, ST_Y(geom) AS gy FROM $table")->fetchAll();
            $this->assertCount(count(self::$attendu[$table]), $rows, "$table : nombre de lignes");
            foreach ($rows as $r) {
                $id = $r['osm_type'] . '/' . $r['osm_id'];
                $this->assertArrayHasKey($id, self::$attendu[$table], "id inattendu $id");
                foreach (self::$attendu[$table][$id] as $col => $val) {
                    if ($col === 'min_zoom') {
                        continue;
                    }
                    if (is_float($val) || (is_int($val) && in_array($col, ['lat', 'lon'], true))) {
                        $this->assertEqualsWithDelta($val, (float) $r[$col], 1e-9, "$table $id $col");
                    } elseif (is_int($val)) {
                        $this->assertSame($val, (int) $r[$col], "$table $id $col");
                    } else {
                        $this->assertSame($val, $r[$col], "$table $id $col (octets: " . bin2hex((string) $r[$col]) . ')');
                    }
                }
                $this->assertEqualsWithDelta((float) $r['lon'], (float) $r['gx'], 1e-12, "$table $id geom.x");
                $this->assertEqualsWithDelta((float) $r['lat'], (float) $r['gy'], 1e-12, "$table $id geom.y");
            }
        }
    }

    public function testSansEffetDeBord(): void
    {
        $this->assertSame(42, (int) self::$pdo->query('SELECT a FROM x')->fetchColumn(), 'table sentinelle intacte');
        $tables = self::$pdo->query('SHOW TABLES')->fetchAll(PDO::FETCH_COLUMN);
        sort($tables);
        $this->assertSame(['osm_lieux', 'osm_stations', 'x'], $tables);
        $this->assertSame(17, (int) self::$pdo->query('SELECT COUNT(*) FROM osm_lieux')->fetchColumn());
    }

    public function testNomsTrapesEtTraitementDesCasLimites(): void
    {
        $nul = self::$pdo->query("SELECT nom FROM osm_lieux WHERE osm_id = 9000000005")->fetchColumn();
        $this->assertSame("nul\0au\0milieu", $nul, 'octets NUL conservés');
        $long = self::$pdo->query('SELECT nom FROM osm_lieux WHERE osm_id = 9100000000')->fetchColumn();
        $this->assertSame(str_repeat('L', 255), $long, 'nom de 5000 car. tronqué à 255, sans erreur');
        $vide = self::$pdo->query('SELECT nom, nom_alt FROM osm_lieux WHERE osm_id = 9000000012')->fetch();
        $this->assertSame('', $vide['nom']);
        $this->assertSame('', $vide['nom_alt'], '\'\' reste \'\' (et non NULL)');
        $null = self::$pdo->query('SELECT nom_alt, population FROM osm_lieux WHERE osm_id = 9100000000')->fetch();
        $this->assertNull($null['nom_alt']);
        $this->assertNull($null['population']);
        $litt = self::$pdo->query("SELECT nom FROM osm_lieux WHERE osm_id = 9000000010")->fetchColumn();
        $this->assertSame('\\N', $litt, 'le texte « \\N » n\'est pas un NULL');
    }

    public function testUnNomOsmIdentiqueEstRelu_ParLApi(): void
    {
        $query = [];
        $router = new Router();
        Api::register($router, fn () => self::$conn, 'test', $query);
        $get = function (array $q) use ($router, &$query): array {
            $query = $q;
            return $router->dispatch('GET', '/v1/lieux');
        };
        foreach (["L'apostrophe d'Anne", "'); DROP TABLE x;--", "anti\\slash \\' \\\\ \\0 \\n fin\\", "emoji \u{1F3CD}\u{1F6E2} \u{e9}\u{e8} \u{4e2d}\u{6587} \u{645}\u{631}\u{62d}\u{628}\u{627}"] as $nom) {
            [$code, $body] = $get(['q' => $nom, 'limite' => '5']);
            $this->assertSame(200, $code, $nom);
            $noms = array_column($body['donnees'], 'nom');
            $this->assertContains($nom, $noms, "relu à l'identique : $nom");
            $this->assertSame(json_encode($nom), json_encode($body['donnees'][array_search($nom, $noms, true)]['nom']));
        }
        $this->assertSame(42, (int) self::$pdo->query('SELECT a FROM x')->fetchColumn());
    }

    public function testMinZoomDansLesBornes(): void
    {
        foreach (['osm_lieux', 'osm_stations'] as $table) {
            foreach (self::$pdo->query("SELECT min_zoom FROM $table")->fetchAll(PDO::FETCH_COLUMN) as $z) {
                $this->assertGreaterThanOrEqual(0, (int) $z);
                $this->assertLessThanOrEqual(15, (int) $z);
            }
        }
    }
}
