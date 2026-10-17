<?php
declare(strict_types=1);

require __DIR__ . '/../vendor/autoload.php';

use Carto\{Api, Config, Database, Router};
use Carto\Http\ApiKeyGuard;

$config = Config::load(__DIR__ . '/../.env');
$query = $_GET;
$path = parse_url($_SERVER['REQUEST_URI'] ?? '/', PHP_URL_PATH) ?: '/';
$method = $_SERVER['REQUEST_METHOD'] ?? 'GET';

header('Content-Type: application/json; charset=utf-8');
header('X-Content-Type-Options: nosniff');

$guard = new ApiKeyGuard($config->get('CARTO_API_KEY', ''));
if (!$guard->autorise($_SERVER['HTTP_X_API_KEY'] ?? null)) {
    http_response_code(401);
    header('Cache-Control: no-store');
    echo json_encode(['erreur' => 'jeton absent ou invalide (en-tête X-Api-Key)'], JSON_UNESCAPED_UNICODE);
    exit;
}

$pdo = null;
$router = new Router();
$router->add('GET', '/', fn () => ['service' => 'RuneRide Carto', 'api' => 'v1', 'routes' => ['/v1/sante', '/v1/stations', '/v1/lieux', '/v1/vue'], 'interne' => true]);
Api::register($router, function () use (&$pdo, $config) { return $pdo ??= Database::connect($config); }, $config->get('CARTO_ENV', 'production'), $query);

try {
    [$status, $out] = $router->dispatch($method, $path);
} catch (Throwable $e) { // base indisponible, etc. : jamais de détail technique au client
    error_log('carto: ' . $e);
    [$status, $out] = [503, ['erreur' => 'service indisponible']];
}

http_response_code($status);
// Cache court sur les réponses de données (mêmes requêtes fréquentes, ménage le CPU mutualisé) ; jamais sur la santé ni les erreurs.
$ttl = (int) $config->get('CARTO_CACHE_SECONDS', '300');
$cacheable = $status === 200 && $path !== '/v1/sante' && $ttl > 0 && $path !== '/';
header('Cache-Control: ' . ($cacheable ? "public, max-age=$ttl" : 'no-store'));
if ($cacheable && ($_SERVER['HTTP_X_API_KEY'] ?? '') !== '') {
    header('Vary: X-Api-Key');
}
echo json_encode($out, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES);
