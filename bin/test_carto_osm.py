"""Tests unitaires (sans base) de carto_osm.py : python3 -m unittest discover -s bin -p 'test_*.py'"""
import os
import random
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import carto_osm as c


class Echappement(unittest.TestCase):
    def test_champs(self):
        self.assertEqual(c.champ(None), "\\N")
        self.assertEqual(c.champ("a\tb\nc\rd\\e\0f"), "a\\tb\\nc\\rd\\\\e\\0f")
        self.assertEqual(c.champ("\\N"), "\\\\N")  # le texte « \N » n'est pas un NULL
        self.assertEqual(c.champ(12), "12")
        self.assertEqual(c.ligne(["x", None, 1.5]), "x\t\\N\t1.5\n")

    def test_une_valeur_ne_peut_pas_fabriquer_un_separateur(self):
        for v in c.CHAINES_PIEGES:
            e = c.champ(v)
            self.assertNotIn("\t", e)
            self.assertNotIn("\n", e)
            self.assertNotIn("\r", e)
            self.assertNotIn("\0", e)

    def test_troncature(self):
        n = {}
        self.assertEqual(len(c.tronquer("x" * 300, 255, n)), 255)
        self.assertEqual(c.tronquer("court", 255, n), "court")
        self.assertEqual(n["tronques"], 1)
        self.assertEqual(c.sans_substituts("a\ud800b"), "a?b")


class Regles(unittest.TestCase):
    def test_lieux(self):
        z = c.zoom_base_lieu
        self.assertEqual(z("city", 2_000_000), 0)
        self.assertEqual(z("city", 500_000), 2)
        self.assertEqual(z("city", None), 4)
        self.assertEqual(z("town", 30_000), 6)
        self.assertEqual(z("town", None), 8)
        self.assertEqual(z("village", 3000), 9)
        self.assertEqual(z("village", 100), 11)
        self.assertEqual((z("suburb", None), z("hamlet", None), z("quarter", 5)), (12, 13, 13))
        self.assertEqual(z("inconnu", 1), c.ZOOM_MAX)

    def test_stations(self):
        z = c.zoom_base_station
        self.assertEqual(z("Station", "TotalEnergies Access"), 10)
        self.assertEqual(z(None, "E.Leclerc"), 10)
        self.assertEqual(z("Station Intermarché", None), 10)
        self.assertEqual(z("Garage Dupont", "Dupont Carburants"), 11)
        self.assertEqual(z("Garage Dupont", None), 12)
        self.assertEqual(z("Botanique", None), 12)  # « bp » ne doit pas matcher dans un mot

    def test_tuiles(self):
        self.assertEqual(c.tuile(85.0, -180.0, 0), (0, 0))
        self.assertEqual(c.tuile(0.0, 0.0, 1), (1, 1))


class Ecremage(unittest.TestCase):
    def test_une_seule_par_cellule_et_nesting(self):
        rnd = random.Random(1)
        k = 2
        pts = [(rnd.choice([4, 8, 11, 13]), rnd.randint(0, 1000), 43 + rnd.random() * 2, 1 + rnd.random() * 2)
               for _ in range(3000)]
        mz = c.ecremer(pts, k)
        for z in range(0, c.ZOOM_ECREMAGE_MAX + 1):
            vus = {}
            for (b, imp, la, lo), m in zip(pts, mz):
                self.assertGreaterEqual(m, min(b, c.ZOOM_MAX))
                if m <= z:
                    cle = c.tuile(la, lo, z + k)
                    self.assertNotIn(cle, vus, "deux points visibles dans la même cellule à z=%d" % z)
                    vus[cle] = 1

    def test_le_plus_important_garde_son_zoom(self):
        pts = [(13, 10, 43.0, 1.0), (13, 99, 43.00001, 1.00001), (4, 1, 43.00002, 1.00002)]
        mz = c.ecremer(pts, 2)
        self.assertEqual(mz[2], 4)       # base la plus basse d'abord
        self.assertGreater(mz[1], 13)    # puis la population 99, repoussée par le point z4 jusqu'à ce que les cellules les séparent
        self.assertGreaterEqual(mz[0], mz[1])

    def test_deterministe(self):
        pts = [(13, 0, 43.0, 1.0)] * 5
        self.assertEqual(c.ecremer(pts, 2), c.ecremer(pts, 2))
        self.assertEqual(sorted(c.ecremer(pts, 2)), [13, 15, 15, 15, 15])


if __name__ == "__main__":
    unittest.main()
