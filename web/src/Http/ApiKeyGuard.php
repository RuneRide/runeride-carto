<?php
declare(strict_types=1);

namespace Carto\Http;

/**
 * Jeton partagé facultatif. Si CARTO_API_KEY est défini (non vide), l'en-tête X-Api-Key doit
 * correspondre (comparaison à temps constant). Ce n'est pas un système d'authentification :
 * l'API reste interne à RuneRide.
 */
final class ApiKeyGuard
{
    public function __construct(private readonly string $cle) {}

    public function autorise(?string $fourni): bool
    {
        return $this->cle === '' || ($fourni !== null && hash_equals($this->cle, $fourni));
    }
}
