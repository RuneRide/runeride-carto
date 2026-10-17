<?php
declare(strict_types=1);

namespace Carto\Http;

/**
 * CORS strict : seules les origines listées (CARTO_CORS_ORIGINS, séparées par des virgules) reçoivent des
 * en-têtes d'autorisation, jamais de joker. Sans liste, aucun en-tête CORS n'est émis.
 * Attention : une clé d'API envoyée depuis un navigateur n'est pas secrète ; le CORS ne protège que contre
 * l'usage depuis d'autres sites web, pas contre un client quelconque.
 */
final class Cors
{
    /** @var list<string> */
    private readonly array $origines;

    public function __construct(string $liste)
    {
        $this->origines = array_values(array_filter(array_map(
            fn (string $o) => rtrim(trim($o), '/'),
            explode(',', $liste),
        ), fn (string $o) => $o !== '' && $o !== '*'));
    }

    public function autorise(?string $origine): bool
    {
        return $origine !== null && in_array($origine, $this->origines, true);
    }

    /** @return array<string,string> en-têtes à émettre ; vide si l'origine n'est pas autorisée */
    public function entetes(?string $origine, bool $preflight): array
    {
        if ($this->origines === []) {
            return [];
        }
        $h = ['Vary' => 'Origin'];
        if (!$this->autorise($origine)) {
            return $h;
        }
        $h['Access-Control-Allow-Origin'] = (string) $origine;
        $h['Access-Control-Expose-Headers'] = 'Cache-Control';
        if ($preflight) {
            $h['Access-Control-Allow-Methods'] = 'GET, OPTIONS';
            $h['Access-Control-Allow-Headers'] = 'X-Api-Key, Content-Type';
            $h['Access-Control-Max-Age'] = '86400';
        }
        return $h;
    }
}
