#!/usr/bin/env python3
# SPDX-License-Identifier: EUPL-1.2
"""Convertit des tuiles Copernicus GLO-30 (GeoTIFF float32, 1° × 1°, 3600 × 3600 sous 50° de latitude)
en grilles brutes d'entiers 16 bits (mètres, petit-boutiste), lues telles quelles par `raido-graphe`.

Usage : convertir-mnt.py <dossier_tif> <dossier_sortie>
Entrée  : Copernicus_DSM_COG_10_N45_00_E005_00_DEM.tif (téléchargeables sans compte sur
          https://copernicus-dem-30m.s3.amazonaws.com/<nom>/<nom>.tif)
Sortie  : N45E005.i16, 3600 × 3600 × 2 octets, ligne 0 = nord, colonne 0 = ouest ; pixel (r, c) centré en
          (N + 1 − (r + 0,5)/3600, E + (c + 0,5)/3600). Valeur -32768 = inconnu.
"""
import re
import sys
from pathlib import Path

import numpy as np
from PIL import Image

Image.MAX_IMAGE_PIXELS = None


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    entree, sortie = Path(sys.argv[1]), Path(sys.argv[2])
    sortie.mkdir(parents=True, exist_ok=True)
    n = 0
    for f in sorted(entree.glob("*.tif")):
        m = re.search(r"_([NS])(\d\d)_00_([EW])(\d\d\d)_00_DEM", f.name)
        if not m:
            print("ignoré :", f.name)
            continue
        la = int(m[2]) * (1 if m[1] == "N" else -1)
        lo = int(m[4]) * (1 if m[3] == "E" else -1)
        a = np.array(Image.open(f), dtype=np.float32)
        if a.shape != (3600, 3600):
            print(f"ignoré (taille {a.shape}, attendu 3600 × 3600) : {f.name}")
            continue
        v = np.where(np.isfinite(a), np.clip(np.rint(a), -32767, 32767), -32768).astype("<i2")
        nom = f"{'N' if la >= 0 else 'S'}{abs(la):02d}{'E' if lo >= 0 else 'W'}{abs(lo):03d}.i16"
        (sortie / nom).write_bytes(v.tobytes())
        print(nom, int(v[v > -32768].min()), int(v.max()))
        n += 1
    print(n, "tuile(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
