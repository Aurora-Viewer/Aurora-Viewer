# Constellation data

The stick lines and names of the constellations the viewer draws over the
real night sky, and the planets' names. The JSON files the viewer reads are
built into the executable (`src/world/constellations_builtin.rs`, its
`TABLE`); nothing is read from disk or from the network to show them. No
picture is built in: drawing sets come from the user's own folder only.

```text
lines.json, names.json        the western culture: the 88 constellations' stick lines,
                              name positions and names by language
drafts.json                   which of those names (and kana readings) are drafts made
                              for this viewer, not taken from a source
planets.json                  the planets' names by language
cultures/<id>/lines.json      a sky culture's stick lines, name positions and names
cultures/<id>/names.json      its figures' names by language
cultures/<id>/culture.json    its own name by language
cultures/<id>/drafts.json     its draft names and readings, as above
credits.json                  the sources of each unit (the root's: the western culture
cultures/<id>/credits.json    and the planets), shown in the viewer's "Licenses and
                              attribution" window
provenance.json               the origin marks of each unit (below)
cultures/<id>/provenance.json
index.json                    the cultures (and art sets) built in
```

`index.json`, `provenance.json` and `drafts.json` are records for people:
the viewer does not read them. A test checks that `index.json`, the folders,
the built-in table and the rows of `NOTICE.md` ("Bundled assets") agree,
and that every figure has its origin marks.

## Sources

Each source, what was taken from it, its licence and its address are in
the unit's `credits.json`, and summed up in `NOTICE.md`. In short: the
western lines and Latin names come from d3-celestial (Olaf Frohn,
BSD-3-Clause), their line ends moved onto the nearest catalogue star; the
six cultures from the Stellarium sky cultures at commit
`e233fa9b3d21e2cafe9c168e6710b627532a7dd4` (Chinese, Japanese: CC BY-SA 4.0;
Egyptian, Inuit, Korean: "GNU GPL v2.0" as their descriptions write it;
Norse (Edda): CC BY 4.0), converted to sky positions and separate segments;
the names in other languages from the Stellarium project's translations,
Wikidata, Wikipedia and OpenCC's conversion tables, as each file's credits
say. What has no source (name positions, the strokes' order and timing,
draft names and kana readings) was made for this viewer and is marked so.

## Origin marks

`provenance.json` beside each unit's files marks how each part came to be:

- `sourced`: taken from the credited source, only its format or coordinates
  converted;
- `derived`: computed from sourced data by a fixed rule (a line end moved
  onto the nearest catalogue star, a name position at the centre of the
  figure's segments);
- `adapted`: source content changed by judgement;
- `made`: no source content (name positions placed by hand, the strokes'
  timing, draft names);
- `unknown`: not recorded.

A figure's field (`lines`, `label`, `timing`…) resolves as
`figures[id][field]`, then `default[field]`, then `unknown`; `names` and
`readings` give each language's entry, `"*"` being the file's default name.

## Your own data

The user's folder, `constellations/` in the configuration folder (or the
folder named by `AURORA_CONSTELLATION_DIR` for a launch), has the same
layout, plus `sets/<id>/` for drawing sets. Each unit there replaces the
built-in unit of the same place whole, never figure by figure:

- `cultures/<id>/`: a culture of that id;
- `lines.json` or `names.json` at the root: the western culture (both
  files of the built-in one are left out); a `cultures/western/` folder
  also replaces it;
- `planets.json`: the planets' names.

A folder with no file the viewer reads replaces nothing. The root
`credits.json` of the user's folder adds to the built-in one.
`AURORA_CONSTELLATION_BUILTIN=0` leaves the built-in data out for a launch
(the user's folder alone).
