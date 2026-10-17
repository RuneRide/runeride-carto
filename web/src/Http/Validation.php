<?php
declare(strict_types=1);

namespace Carto\Http;

/** Lecture et validation des paramètres de requête. Lève ApiException (400) avec le nom du paramètre fautif. */
final class Validation
{
    public const RAYON_MAX_KM = 50.0;
    public const RAYON_DEFAUT_KM = 10.0;
    public const LIMITE_MAX = 100;
    public const Q_MIN = 2;
    public const Q_MAX = 64;

    /** @param array<string,mixed> $q */
    public static function latitude(array $q, string $nom = 'lat', bool $requis = true): ?float
    {
        return self::nombre($q, $nom, -90.0, 90.0, $requis);
    }

    /** @param array<string,mixed> $q */
    public static function longitude(array $q, string $nom = 'lon', bool $requis = true): ?float
    {
        return self::nombre($q, $nom, -180.0, 180.0, $requis);
    }

    /** @param array<string,mixed> $q rayon en km */
    public static function rayonKm(array $q): float
    {
        $r = self::nombre($q, 'rayon', 0.0, self::RAYON_MAX_KM, false);
        if ($r !== null && $r <= 0.0) {
            throw new ApiException(400, 'rayon doit être supérieur à 0', 'rayon');
        }
        return $r ?? self::RAYON_DEFAUT_KM;
    }

    /** @param array<string,mixed> $q */
    public static function limite(array $q, int $defaut): int
    {
        $v = $q['limite'] ?? null;
        if ($v === null || $v === '') {
            return $defaut;
        }
        if (!is_string($v) || !preg_match('/^\d{1,4}$/', $v) || (int) $v < 1 || (int) $v > self::LIMITE_MAX) {
            throw new ApiException(400, 'limite doit être un entier entre 1 et ' . self::LIMITE_MAX, 'limite');
        }
        return (int) $v;
    }

    /** @param array<string,mixed> $q */
    public static function marque(array $q): ?string
    {
        $v = $q['marque'] ?? null;
        if ($v === null || $v === '') {
            return null;
        }
        if (!is_string($v) || !mb_check_encoding($v, 'UTF-8') || mb_strlen(trim($v)) > 64) {
            throw new ApiException(400, 'marque invalide (texte de 64 caractères au plus)', 'marque');
        }
        return trim($v);
    }

    /** @param array<string,mixed> $q */
    public static function prefixe(array $q): string
    {
        $v = $q['q'] ?? null;
        if (!is_string($v) || !mb_check_encoding($v, 'UTF-8')) {
            throw new ApiException(400, 'q est requis (texte)', 'q');
        }
        $v = trim($v);
        $n = mb_strlen($v);
        if ($n < self::Q_MIN || $n > self::Q_MAX) {
            throw new ApiException(400, sprintf('q doit faire entre %d et %d caractères', self::Q_MIN, self::Q_MAX), 'q');
        }
        return $v;
    }

    /** @param array<string,mixed> $q */
    private static function nombre(array $q, string $nom, float $min, float $max, bool $requis): ?float
    {
        $v = $q[$nom] ?? null;
        if ($v === null || $v === '') {
            return $requis ? throw new ApiException(400, "$nom est requis", $nom) : null;
        }
        if (!is_string($v) || !preg_match('/^-?\d+(\.\d+)?$/', $v)) {
            throw new ApiException(400, "$nom doit être un nombre décimal", $nom);
        }
        $f = (float) $v;
        if ($f < $min || $f > $max) {
            throw new ApiException(400, "$nom doit être entre $min et $max", $nom);
        }
        return $f;
    }
}
