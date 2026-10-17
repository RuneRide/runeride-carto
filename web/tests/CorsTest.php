<?php
declare(strict_types=1);

namespace Carto\Tests;

use Carto\Http\Cors;
use PHPUnit\Framework\TestCase;

final class CorsTest extends TestCase
{
    public function testOrigineAutoriseeRecoitLesEntetes(): void
    {
        $h = (new Cors('https://app.runeride.fr'))->entetes('https://app.runeride.fr', false);
        $this->assertSame('https://app.runeride.fr', $h['Access-Control-Allow-Origin']);
        $this->assertSame('Origin', $h['Vary']);
        $this->assertArrayNotHasKey('Access-Control-Allow-Methods', $h);
    }

    public function testPreflightAjouteMethodesEtEntetes(): void
    {
        $h = (new Cors('https://app.runeride.fr'))->entetes('https://app.runeride.fr', true);
        $this->assertStringContainsString('X-Api-Key', $h['Access-Control-Allow-Headers']);
        $this->assertSame('GET, OPTIONS', $h['Access-Control-Allow-Methods']);
    }

    public function testOrigineInconnueNeRecoitAucuneAutorisation(): void
    {
        $c = new Cors('https://app.runeride.fr');
        $this->assertArrayNotHasKey('Access-Control-Allow-Origin', $c->entetes('https://evil.example', false));
        $this->assertArrayNotHasKey('Access-Control-Allow-Origin', $c->entetes('https://app.runeride.fr.evil.example', false));
        $this->assertArrayNotHasKey('Access-Control-Allow-Origin', $c->entetes(null, false));
        $this->assertFalse($c->autorise('http://app.runeride.fr'));
    }

    public function testJokerEtListeVideNeDonnentRien(): void
    {
        $this->assertSame([], (new Cors(''))->entetes('https://app.runeride.fr', false));
        $joker = new Cors('*');
        $this->assertSame([], $joker->entetes('https://app.runeride.fr', false));
        $this->assertFalse($joker->autorise('*'));
    }

    public function testPlusieursOriginesEtSlashFinal(): void
    {
        $c = new Cors(' https://app.runeride.fr/ , https://runeride.fr ');
        $this->assertTrue($c->autorise('https://app.runeride.fr'));
        $this->assertTrue($c->autorise('https://runeride.fr'));
        $this->assertFalse($c->autorise('https://carto.runeride.fr'));
    }
}
