# Night sky stars

The viewer draws its night sky from two files, built into the executable
(`crates/aurora-render/src/stars.rs`):

- `stars.bin`: the 8,795 stars of visual magnitude 6.5 or brighter (the
  stars visible to the naked eye under a dark sky): right ascension,
  declination, magnitude and B-V colour index;
- `milky_way.bin`: the Milky Way, from the 2.5 million fainter stars: on a
  grid of galactic longitude and latitude (0.18° per cell, within 45° of
  the galactic plane), the light of the stars of magnitude 9.5 and fainter,
  their colour, and the count of the stars of magnitude 6.5 to 9.5 that
  the sky shader scatters as faint points.

Both are derived from the **Tycho-2 Catalogue** (Høg E., Fabricius C.,
Makarov V.V., Urban S., Corbin T., Wycoff G., Bastian U., Schwekendiek P.,
Wicenec A., 2000, A&A 355, L27), its main catalogue and first supplement,
from the copy at https://archive.eso.org/ASTROM/TYC-2/data/ (also CDS
I/259). Its lead author agreed to the BSD-2-Clause licence in 2015
([LICENSE-TYCHO2.md](LICENSE-TYCHO2.md)).

Tycho-2's photometry saturates on the brightest stars (none brighter than
VT 1.9 in its main catalogue, only a Hipparcos Hp magnitude without colour
in its supplement). For the 50 stars brighter than V 2.0 or without a
colour index, the magnitude V and the B-V index come from the **Hipparcos
Catalogue** (ESA 1997, *The Hipparcos and Tycho Catalogues*, ESA SP-1200),
read from HEASARC's copy (`heasarc_hipparcos.tdat.gz`); the other Tycho-2
components of those stars' systems are left out, as Hipparcos gives the
system's combined light.

Changes: Tycho BT and VT turned into Johnson V and B-V (ESA SP-1200, vol. 1,
sect. 1.3); the naked-eye stars sorted by magnitude, their position,
magnitude and colour rounded to fixed-point values; the fainter stars
counted on the grid, smoothed, less their mean count far from the galactic
plane, and weighted towards the inner disk and the bulge for the light of
the stars too far for Tycho-2 (the generator explains it).

To rebuild both:

```powershell
python scripts/generate-star-catalog.py <folder with catalog.dat and suppl_1.dat> heasarc_hipparcos.tdat.gz assets/stars
```
