<?php
// SPDX-License-Identifier: EUPL-1.2
// Fabrique, avec la classe Jwt du PHP de app.runeride.fr (runeride/web/src/Auth/Jwt.php), une clé de session, son JWKS
// et des jetons courts au format de POST /api/balade/jeton (BaladeApi::token), pour les rejouer contre la Lambda Raido
// en local. Aucun réseau, aucun secret réel : la clé est engendrée ici et jetée après l'essai.
//
//   php jetons.php <chemin de runeride/web/src/Auth/Jwt.php> <dossier de sortie>
declare(strict_types=1);
[$jwtPhp, $dir] = [$argv[1] ?? '', $argv[2] ?? ''];
if (!is_file($jwtPhp) || $dir === '') {
    fwrite(STDERR, "usage : php jetons.php <runeride/web/src/Auth/Jwt.php> <dossier de sortie>\n");
    exit(2);
}
require $jwtPhp;
$pem = RuneRide\Auth\Jwt::generateKeyPem();
$iss = 'https://app.test';                                   // RUNERIDE_BASE_URL : l'émetteur (iss) des jetons
$session = new RuneRide\Auth\Jwt($pem, $iss, 'runeride');    // exactement l'objet $jwt de public/index.php
@mkdir($dir, 0700, true);
file_put_contents("$dir/jwks.json", json_encode($session->jwks()));  // ce que sert GET /.well-known/jwks.json
$sub = 'u' . substr(hash('sha256', 'raido-sujet:42'), 0, 24);           // BaladeApi::subject(42) : sujet opaque
$now = time();
$claims = fn (string $palier, string $type) => ['palier' => $palier, 'type' => $type, 'limite_km' => 25];
$jetons = [
    'gratuit_petite_boucle' => $session->issueFor($sub, 'raido', $claims('gratuit', 'petite_boucle'), 900, $now),
    'gratuit_grand' => $session->issueFor($sub, 'raido', $claims('gratuit', 'grand'), 900, $now),
    'abonne_grand' => $session->issueFor($sub, 'raido', $claims('abonne', 'grand'), 900, $now),
    'session_runeride' => $session->issue(42),                                      // jeton de session (aud = runeride)
    'expire' => $session->issueFor($sub, 'raido', $claims('abonne', 'grand'), 60, $now - 3600),
    'autre_cle' => (new RuneRide\Auth\Jwt(RuneRide\Auth\Jwt::generateKeyPem(), $iss, 'runeride'))->issueFor($sub, 'raido', $claims('abonne', 'grand'), 900, $now),
];
file_put_contents("$dir/jetons.json", json_encode($jetons, JSON_PRETTY_PRINT));
echo "JWKS et " . count($jetons) . " jetons écrits dans $dir\n";
