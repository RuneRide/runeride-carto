<?php
declare(strict_types=1);

namespace Carto\Tests;

use Carto\Http\{ApiException, Validation};
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class ValidationTest extends TestCase
{
    public function testValeursValides(): void
    {
        $this->assertSame(43.6, Validation::latitude(['lat' => '43.6']));
        $this->assertSame(-1.5, Validation::longitude(['lon' => '-1.5']));
        $this->assertSame(10.0, Validation::rayonKm([]));
        $this->assertSame(50.0, Validation::rayonKm(['rayon' => '50']));
        $this->assertSame(50, Validation::limite([], 50));
        $this->assertSame(100, Validation::limite(['limite' => '100'], 50));
        $this->assertNull(Validation::latitude([], 'lat', false));
        $this->assertSame('Toulou', Validation::prefixe(['q' => ' Toulou ']));
    }

    /** @return iterable<string,array{0:callable,1:string}> */
    public static function invalides(): iterable
    {
        yield 'lat absente' => [fn () => Validation::latitude([]), 'lat'];
        yield 'lat > 90' => [fn () => Validation::latitude(['lat' => '90.1']), 'lat'];
        yield 'lat texte' => [fn () => Validation::latitude(['lat' => 'abc']), 'lat'];
        yield 'lat NaN' => [fn () => Validation::latitude(['lat' => 'NAN']), 'lat'];
        yield 'lat tableau' => [fn () => Validation::latitude(['lat' => ['1']]), 'lat'];
        yield 'lon < -180' => [fn () => Validation::longitude(['lon' => '-180.5']), 'lon'];
        yield 'lon notation scientifique' => [fn () => Validation::longitude(['lon' => '1e2']), 'lon'];
        yield 'rayon > 50' => [fn () => Validation::rayonKm(['rayon' => '50.1']), 'rayon'];
        yield 'rayon 0' => [fn () => Validation::rayonKm(['rayon' => '0']), 'rayon'];
        yield 'rayon négatif' => [fn () => Validation::rayonKm(['rayon' => '-3']), 'rayon'];
        yield 'limite 101' => [fn () => Validation::limite(['limite' => '101'], 50), 'limite'];
        yield 'limite 0' => [fn () => Validation::limite(['limite' => '0'], 50), 'limite'];
        yield 'limite décimale' => [fn () => Validation::limite(['limite' => '5.5'], 50), 'limite'];
        yield 'q absent' => [fn () => Validation::prefixe([]), 'q'];
        yield 'q trop court' => [fn () => Validation::prefixe(['q' => 'a']), 'q'];
        yield 'q trop long' => [fn () => Validation::prefixe(['q' => str_repeat('a', 65)]), 'q'];
        yield 'q UTF-8 invalide' => [fn () => Validation::prefixe(['q' => "ab\xff"]), 'q'];
        yield 'marque trop longue' => [fn () => Validation::marque(['marque' => str_repeat('x', 65)]), 'marque'];
    }

    #[DataProvider('invalides')]
    public function testRejets(callable $appel, string $parametre): void
    {
        try {
            $appel();
            $this->fail('ApiException attendue');
        } catch (ApiException $e) {
            $this->assertSame(400, $e->status);
            $this->assertSame($parametre, $e->parametre);
        }
    }
}
