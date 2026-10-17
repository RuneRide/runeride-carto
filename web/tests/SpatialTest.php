<?php
declare(strict_types=1);

namespace Carto\Tests;

use Carto\Repository\Spatial;
use PHPUnit\Framework\TestCase;

final class SpatialTest extends TestCase
{
    public function testBoiteEtPoint(): void
    {
        $s = new Spatial();
        $this->assertSame('POINT(1.4400000 43.6000000)', $s->pointWkt(43.6, 1.44));
        $this->assertMatchesRegularExpression('/^POLYGON\(\(.*\)\)$/', $s->boiteWkt(43.6, 1.44, 10000));
        // Aux pôles et au bord : bornes respectées
        $this->assertStringNotContainsString('-181', $s->boiteWkt(0.0, -179.99, 50000));
        $this->assertStringNotContainsString(' 91', $s->boiteWkt(89.99, 0.0, 50000));
    }
}
