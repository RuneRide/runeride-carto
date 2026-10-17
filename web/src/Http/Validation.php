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
    public const ZOOM_MAX = 22;
    /** /v1/vue : taille de boîte maximale (degrés), au plus BBOX_TUILES tuiles de large à ce zoom, et limite totale. */
    public const BBOX_MAX_LON = 30.0;
    public const BBOX_MAX_LAT = 20.0;
    public const BBOX_TUILES = 8;
    public const VUE_LIMITE_DEFAUT = 200;
    public const VUE_LIMITE_MAX = 500;

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
    public static function limite(array $q, int $defaut, int $max = self::LIMITE_MAX): int
    {
        $v = $q['limite'] ?? null;
        if ($v === null || $v === '') {
            return $defaut;
        }
        if (!is_string($v) || !preg_match('/^\d{1,4}$/D', $v) || (int) $v < 1 || (int) $v > $max) {
            throw new ApiException(400, 'limite doit être un entier entre 1 et ' . $max, 'limite');
        }
        return (int) $v;
    }

    /** Zoom entier 0..22 (le niveau de zoom de la carte du client). @param array<string,mixed> $q */
    public static function zoom(array $q, bool $requis = false): ?int
    {
        $v = $q['zoom'] ?? null;
        if ($v === null || $v === '') {
            return $requis ? throw new ApiException(400, 'zoom est requis', 'zoom') : null;
        }
        if (!is_string($v) || !preg_match('/^\d{1,2}$/D', $v) || (int) $v > self::ZOOM_MAX) {
            throw new ApiException(400, 'zoom doit être un entier entre 0 et ' . self::ZOOM_MAX, 'zoom');
        }
        return (int) $v;
    }

    /**
     * bbox=ouest,sud,est,nord (degrés décimaux). Pas d'antiméridien : ouest < est, sud < nord ;
     * taille bornée : au plus BBOX_MAX_LON x BBOX_MAX_LAT degrés ET au plus BBOX_TUILES tuiles de 256 px
     * de côté au zoom demandé (une carte plus grande que cela n'a pas besoin d'une seule requête).
     * @param array<string,mixed> $q
     * @return array{0:float,1:float,2:float,3:float}
     */
    public static function bbox(array $q, int $zoom): array
    {
        $v = $q['bbox'] ?? null;
        if (!is_string($v) || $v === '') {
            throw new ApiException(400, 'bbox est requis (ouest,sud,est,nord)', 'bbox');
        }
        $parts = explode(',', $v);
        if (count($parts) !== 4) {
            throw new ApiException(400, 'bbox doit avoir 4 valeurs : ouest,sud,est,nord', 'bbox');
        }
        foreach ($parts as $p) {
            if (!preg_match('/^-?\d{1,3}(\.\d{1,10})?$/D', $p)) {
                throw new ApiException(400, 'bbox : chaque valeur doit être un nombre décimal', 'bbox');
            }
        }
        [$o, $s, $e, $n] = array_map('floatval', $parts);
        if ($o < -180.0 || $e > 180.0 || $s < -90.0 || $n > 90.0) {
            throw new ApiException(400, 'bbox hors limites (lon -180..180, lat -90..90)', 'bbox');
        }
        if ($o >= $e || $s >= $n) {
            throw new ApiException(400, 'bbox : ouest < est et sud < nord requis (antiméridien non géré)', 'bbox');
        }
        $tuilesDeg = self::BBOX_TUILES * 360.0 / (1 << $zoom);
        $maxLon = min(self::BBOX_MAX_LON, $tuilesDeg);
        $maxLat = min(self::BBOX_MAX_LAT, $tuilesDeg);
        if ($e - $o > $maxLon || $n - $s > $maxLat) {
            throw new ApiException(400, sprintf('bbox trop grande à ce zoom (au plus %.4g° de large et %.4g° de haut)', $maxLon, $maxLat), 'bbox');
        }
        return [$o, $s, $e, $n];
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
        if (!is_string($v) || !preg_match('/^-?\d+(\.\d+)?$/D', $v)) {
            throw new ApiException(400, "$nom doit être un nombre décimal", $nom);
        }
        $f = (float) $v;
        if ($f < $min || $f > $max) {
            throw new ApiException(400, "$nom doit être entre $min et $max", $nom);
        }
        return $f;
    }
}
