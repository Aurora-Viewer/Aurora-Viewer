# Swimming animations

Swimming pack contributed by the project maintainer, retained for local
and offline previews in Aurora Viewer. Original QAvimator BVH files are preserved here.
The loader removes root translations, closes the loops and orients the
backwards stroke for head-first travel. No personal source path is embedded.

The six clips cover forward strokes, backwards swimming, surface floating,
underwater floating and diving. Local rendering works without an upload.
These clips remain for local/offline previews. There is no bundled-pack upload
button, startup upload request or automatic seeding of AO states from this pack.
Shared swimming animations are selected by the ordinary AO from inventory assets.


`cannonball.anim` is a separately authored native body clip for explicit offline
water-action previews. It is never selected as a private online fallback.
Priority 6, 21 standard body joints, 2.8 seconds, intro through 0.6 s, a constant
0.6–1.2 s tuck loop, then recovery into the supplied underwater idle pose.
Only the pelvis has a small local translation; no world trajectory, facial or
Bento finger channels, scale tracks or constraints are encoded. The animation
holds knees to the torso with hands around the shins, then opens in water.

Regenerate with Python, numpy and scipy:

```text
python scripts/author-water-cannonball.py --skeleton <pack>/sources/skeleton.json --idle <pack>/05-flotter-sous-eau-P6.anim --output crates/aurora-viewer/assets/animations/swimming/cannonball.anim --report .scratch/cannonball-validation.json
```

The script reads source data only, writes an animation plus a validation report,
and never imports, uploads or edits inventory. The maintainer imports any desired
animation themselves and assigns it in their AO for later grid validation.
