# Creators' LUT packs

The photo mode's LUT tab shows, above the user's own tables, LUT packs
that their creators agreed to share with every Aurora user. Each pack is
built into the executable (`crates/aurora-viewer/src/snapshot/lut_packs.rs`)
and credited under its creator's name in the tab.

## Czar Fenris

`czar-fenris.bin`: 20 looks by **Czar Fenris**, made in Adobe Premiere Pro
and DaVinci Resolve (Beach Vibes, Black and White, Cowboy Country,
Desserted, Destaurated, Film look, Good Skin, Influencer, Movie, Natural
Light, Neutral Skin, Noir, Old Newspaper, Phone Filter, Rave, Soft Skin,
Thunderstorm, Vintage Magazine, Vintage Vodka, Winter Fog).

Czar Fenris agreed on 2026-11-10 that the viewer includes them and shares
them freely with its users, credited under the avatar name Czar Fenris.
The looks remain Czar Fenris' work. The creator's page:
https://linktr.ee/animositysl

"Old Newspaper" is the version Czar Fenris sent on 2026-11-10 in place of
the one of their first folder, which was mixed up with an editor's preset
(its 64³ table is left out).

Changes: the "Czar - " prefix of the file names is left out (the tab's
heading names them); each table's colours are clamped to 0..1 and stored
compressed as 8-bit values (the photo's look samples an 8-bit texture of
0..1, so the look is unchanged). The pack was first stored as 16-bit
values (AURLUTS1); its current file was converted from that one, colour
for colour the same in the texture.

## Nanner Pudding

`nanner-pudding.bin`: 45 looks by **Nanner Pudding**, made in DaVinci
Resolve (Amethyst, Amethyst Dark, Amethyst Light, Analog Horror, Analoge
Horror BW, Angel, Apocalyptic, Baddie, Baddie 1, Bleeding Edge, Blown Out,
Bright and Saturated, Candlewick, Candy Beach, Chilly, Cinnamon, Coconut
Water, Cold Night, Coraline, Crispy, Cybercore, Damned, Dawg Days, Dead
Dove, Fallen Angel, Final Girl, Flip Phone, Florida Man, Frutiger Aero,
Golden Hour, Golden Hour Brighter, Golden Years, Hoa Hoa, Insidious,
Meadow, Natural Night, Night Landscape, San Andreas, So Pink So Pretty,
Tis Autumn, Vintage, Wake Me Up (wake me up inside), Washed Out, Winter
Wonderland, Y2K).

Nanner Pudding agreed on 2026-11-10 that the viewer team includes and
distributes them as part of the viewer, credited under the name Nanner
Pudding, and kept the names of the looks, "Flip Phone" shortened by them.
The looks remain Nanner Pudding's work.

Changes: the "NP - " prefix of the file names is left out; the colours are
stored as for the pack above (33³ tables, "Tis Autumn" 65³).

## Rebuilding a pack

From the creator's folder of .cube files (for Czar Fenris' pack, with
their "Old Newspaper" replaced by the version they sent):

```powershell
python scripts/generate-photo-luts.py "<folder>" assets/photo-luts/czar-fenris.bin --prefix "Czar - "
python scripts/generate-photo-luts.py "<folder>" assets/photo-luts/nanner-pudding.bin --prefix "NP - "
```

The script also converts a pack of the older 16-bit format: give it that
file in place of the folder.

A new pack: generate its file here, add it to `FILES` in `lut_packs.rs`,
then credit it in this README and in NOTICE.md.
