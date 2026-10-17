<?php
declare(strict_types=1);

namespace Carto;

use Doctrine\DBAL\{Connection, DriverManager};
use InvalidArgumentException;
use PDO;

final class Database
{
    /** Pilote DBAL selon le préfixe du DSN PDO (la configuration `.env` reste en DSN PDO). */
    private const PILOTES = ['mysql' => 'pdo_mysql', 'pgsql' => 'pdo_pgsql'];

    /** Connexion DBAL paresseuse : rien n'est ouvert avant la première requête. */
    public static function connect(Config $c): Connection
    {
        return DriverManager::getConnection(self::parametres($c->get('CARTO_DB_DSN'), $c->get('CARTO_DB_USER'), $c->get('CARTO_DB_PASSWORD', '')));
    }

    /**
     * Traduit un DSN PDO (« mysql:host=…;port=…;dbname=…;charset=… ») en paramètres DBAL.
     * @return array<string,mixed>
     */
    public static function parametres(string $dsn, string $user, string $password): array
    {
        [$prefixe, $reste] = array_pad(explode(':', $dsn, 2), 2, '');
        if (!isset(self::PILOTES[$prefixe])) {
            throw new InvalidArgumentException('DSN non pris en charge : pilote ' . $prefixe);
        }
        $kv = [];
        foreach (array_filter(explode(';', $reste), 'strlen') as $part) {
            [$k, $v] = array_pad(explode('=', $part, 2), 2, '');
            $kv[strtolower(trim($k))] = trim($v);
        }
        $p = ['driver' => self::PILOTES[$prefixe], 'user' => $user, 'password' => $password,
            'driverOptions' => [PDO::ATTR_EMULATE_PREPARES => false, PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC]];
        foreach (['host', 'dbname', 'charset', 'unix_socket'] as $cle) {
            if (isset($kv[$cle])) {
                $p[$cle] = $kv[$cle];
            }
        }
        if (isset($kv['port'])) {
            $p['port'] = (int) $kv['port'];
        }
        return $p;
    }
}
