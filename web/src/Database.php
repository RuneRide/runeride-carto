<?php
declare(strict_types=1);

namespace Carto;

use PDO;

final class Database
{
    public static function connect(Config $c): PDO
    {
        return new PDO($c->get('CARTO_DB_DSN'), $c->get('CARTO_DB_USER'), $c->get('CARTO_DB_PASSWORD', ''), [
            PDO::ATTR_ERRMODE => PDO::ERRMODE_EXCEPTION,
            PDO::ATTR_DEFAULT_FETCH_MODE => PDO::FETCH_ASSOC,
            PDO::ATTR_EMULATE_PREPARES => false,
        ]);
    }
}
