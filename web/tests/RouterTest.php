<?php
declare(strict_types=1);

namespace Carto\Tests;

use Carto\Http\{ApiException, ApiKeyGuard};
use Carto\Router;
use PHPUnit\Framework\TestCase;

final class RouterTest extends TestCase
{
    public function testRoutes(): void
    {
        $r = new Router();
        $r->add('GET', '/a', fn () => ['ok' => true]);
        $this->assertSame([200, ['ok' => true]], $r->dispatch('GET', '/a'));
        $this->assertSame(404, $r->dispatch('GET', '/b')[0]);
        $this->assertSame(405, $r->dispatch('POST', '/a')[0]);
    }

    public function testErreurClientEnJson(): void
    {
        $r = new Router();
        $r->add('GET', '/a', fn () => throw new ApiException(400, 'lat est requis', 'lat'));
        $this->assertSame([400, ['erreur' => 'lat est requis', 'parametre' => 'lat']], $r->dispatch('GET', '/a'));
    }

    public function testJetonFacultatif(): void
    {
        $this->assertTrue((new ApiKeyGuard(''))->autorise(null));
        $g = new ApiKeyGuard('secret');
        $this->assertTrue($g->autorise('secret'));
        $this->assertFalse($g->autorise('autre'));
        $this->assertFalse($g->autorise(''));
        $this->assertFalse($g->autorise(null));
    }
}
