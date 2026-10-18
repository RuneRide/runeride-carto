<?php
// Lecture d'une plage d'un fichier PMTiles : ?f=<nom sans extension>&o=<position>&l=<longueur>[&v=<version>].
//
// Pourquoi : sur o2switch, le serveur frontal traite lui-même les requêtes Range des fichiers statiques et lit le
// fichier depuis le début jusqu'à la position demandée (mesuré : 0,3 s à 120 Mo, 3,7 s à 2,4 Go, même en relecture).
// Il ne transmet pas non plus l'en-tête Range à PHP, d'où la position en paramètres. fseek lit directement au bon
// endroit (mesuré : 0,08 à 0,13 s partout). Chaque réponse est complète (200) et immuable : le navigateur la met en cache.
declare(strict_types=1);

header('Access-Control-Allow-Origin: *');
header('X-Content-Type-Options: nosniff');

$nom = (string) ($_GET['f'] ?? '');
$o = filter_input(INPUT_GET, 'o', FILTER_VALIDATE_INT);
$l = filter_input(INPUT_GET, 'l', FILTER_VALIDATE_INT);
if (!preg_match('/^[a-z0-9-]{1,64}$/', $nom) || $o === false || $o === null || $l === false || $l === null || $o < 0 || $l < 1 || $l > 8 * 1024 * 1024) {
    http_response_code(400);
    exit;
}
$fichier = __DIR__ . '/' . $nom . '.pmtiles';
$taille = is_file($fichier) ? filesize($fichier) : false;
if ($taille === false) {
    http_response_code(404);
    exit;
}
if ($o >= $taille) {
    http_response_code(416);
    header('Content-Range: bytes */' . $taille);
    exit;
}
$l = min($l, $taille - $o);
$h = fopen($fichier, 'rb');
if ($h === false || fseek($h, $o) !== 0) {
    http_response_code(500);
    exit;
}
header('Content-Type: application/octet-stream');
header('Content-Length: ' . $l);
// L'adresse change avec la version (?v=) quand le fichier est remplacé : le contenu d'une même adresse ne change jamais.
header('Cache-Control: public, max-age=31536000, immutable');
echo fread($h, $l);
fclose($h);
