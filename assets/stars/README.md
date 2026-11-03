# Night sky stars

`stars.bin` holds the 8,920 stars of visual magnitude 6.5 or brighter (the
stars visible to the naked eye under a dark sky): right ascension,
declination, magnitude and B-V colour index. The viewer draws its night sky
from it (`crates/aurora-render/src/stars.rs`).

It is derived from the **HYG database v4.4** by David Nash (astronexus),
https://codeberg.org/astronexus/hyg, licensed under the
[Creative Commons Attribution-ShareAlike 4.0 International License](https://creativecommons.org/licenses/by-sa/4.0/).
`stars.bin` is distributed under the same licence. Changes: only the stars of
magnitude 6.5 or brighter are kept, without the Sun, sorted by magnitude, and
their position, magnitude and colour index are rounded to fixed-point values.

To rebuild it from a newer release of the database:

```powershell
python scripts/generate-star-catalog.py hyg_v44.csv.gz assets/stars/stars.bin
```
