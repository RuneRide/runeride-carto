<?php
declare(strict_types=1);

namespace Carto\Http;

/** Erreur client (4xx) renvoyée telle quelle en JSON. */
final class ApiException extends \RuntimeException
{
    public function __construct(public readonly int $status, string $message, public readonly ?string $parametre = null)
    {
        parent::__construct($message);
    }
}
