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
        $this->assertNull(Validation::zoom([]));
        $this->assertSame(0, Validation::zoom(['zoom' => '0']));
        $this->assertSame(22, Validation::zoom(['zoom' => '22']));
        $this->assertSame(500, Validation::limite(['limite' => '500'], 200, Validation::VUE_LIMITE_MAX));
        $this->assertSame([1.0, 43.0, 1.5, 43.5], Validation::bbox(['bbox' => '1,43,1.5,43.5'], 10));
        $this->assertSame([-5.5, 41.0, 9.0, 51.5], Validation::bbox(['bbox' => '-5.5,41,9,51.5'], 4));
    }

    public function testTailleDeBoiteSelonLeZoom(): void
    {
        // 8 tuiles de large : 360*8/2^z degrés (z15 : ~0,0879°), plafonné à 30 x 20
        Validation::bbox(['bbox' => '1,43,1.08,43.08'], 15);
        $this->expectException(ApiException::class);
        Validation::bbox(['bbox' => '1,43,1.09,43.05'], 15);
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
        yield 'zoom 23' => [fn () => Validation::zoom(['zoom' => '23']), 'zoom'];
        yield 'zoom négatif' => [fn () => Validation::zoom(['zoom' => '-1']), 'zoom'];
        yield 'zoom décimal' => [fn () => Validation::zoom(['zoom' => '4.5']), 'zoom'];
        yield 'zoom texte' => [fn () => Validation::zoom(['zoom' => '4; DROP TABLE x']), 'zoom'];
        yield 'zoom tableau' => [fn () => Validation::zoom(['zoom' => ['4']]), 'zoom'];
        yield 'zoom requis' => [fn () => Validation::zoom([], true), 'zoom'];
        yield 'limite vue 501' => [fn () => Validation::limite(['limite' => '501'], 200, Validation::VUE_LIMITE_MAX), 'limite'];
        yield 'bbox absente' => [fn () => Validation::bbox([], 5), 'bbox'];
        yield 'bbox 3 valeurs' => [fn () => Validation::bbox(['bbox' => '1,2,3'], 5), 'bbox'];
        yield 'bbox 5 valeurs' => [fn () => Validation::bbox(['bbox' => '1,2,3,4,5'], 5), 'bbox'];
        yield 'bbox texte' => [fn () => Validation::bbox(['bbox' => "1,2,3,4'); DROP TABLE x;--"], 5), 'bbox'];
        yield 'bbox tableau' => [fn () => Validation::bbox(['bbox' => ['1,2,3,4']], 5), 'bbox'];
        yield 'bbox exposant' => [fn () => Validation::bbox(['bbox' => '1e0,2,3,4'], 5), 'bbox'];
        yield 'bbox ouest >= est' => [fn () => Validation::bbox(['bbox' => '3,2,1,4'], 5), 'bbox'];
        yield 'bbox sud >= nord' => [fn () => Validation::bbox(['bbox' => '1,4,3,2'], 5), 'bbox'];
        yield 'bbox hors limites' => [fn () => Validation::bbox(['bbox' => '-181,0,0,1'], 5), 'bbox'];
        yield 'bbox lat hors limites' => [fn () => Validation::bbox(['bbox' => '0,0,1,91'], 5), 'bbox'];
        yield 'bbox trop large' => [fn () => Validation::bbox(['bbox' => '0,0,31,1'], 0), 'bbox'];
        yield 'bbox trop haute' => [fn () => Validation::bbox(['bbox' => '0,0,1,21'], 0), 'bbox'];
        yield 'bbox trop grande pour le zoom' => [fn () => Validation::bbox(['bbox' => '0,40,20,45'], 8), 'bbox'];
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
