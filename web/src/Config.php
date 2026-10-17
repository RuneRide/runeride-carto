<?php
declare(strict_types=1);

namespace Carto;

final class Config
{
    /** @param array<string,string> $values */
    public function __construct(private readonly array $values) {}

    /** Lit un fichier .env (KEY=VALUE) puis laisse l'environnement du processus (CARTO_*) l'emporter. */
    public static function load(string $envFile): self
    {
        $values = [];
        if (is_file($envFile)) {
            foreach (file($envFile, FILE_IGNORE_NEW_LINES | FILE_SKIP_EMPTY_LINES) as $line) {
                $line = trim($line);
                if ($line === '' || $line[0] === '#' || !str_contains($line, '=')) {
                    continue;
                }
                [$k, $v] = explode('=', $line, 2);
                $values[trim($k)] = trim($v, " \t\"'");
            }
        }
        foreach (getenv() as $k => $v) {
            if (str_starts_with($k, 'CARTO_')) {
                $values[$k] = $v;
            }
        }
        return new self($values);
    }

    public function get(string $key, ?string $default = null): string
    {
        return $this->values[$key] ?? $default
            ?? throw new \RuntimeException("Configuration manquante : $key");
    }
}
