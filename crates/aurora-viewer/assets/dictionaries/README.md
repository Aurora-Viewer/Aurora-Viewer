# Spelling dictionaries

Hunspell dictionaries of the spell checker (`src/spelling.rs`), the list of
the Legacy Viewer (`app_settings/dictionaries/dictionaries.xml`). build.rs
gzips them into the executable; the text files stay here so that a new
version is a readable diff.

All but the glossary come unchanged (renamed `<name>.aff` / `<name>.dic`)
from [wooorm/dictionaries](https://github.com/wooorm/dictionaries), commit
`8cfea406b505e4d7df52d5a19bce525df98c54ab` (2024-09-09), which collects
the dictionaries of their upstream projects in UTF-8. Each folder keeps
the licence text of its dictionary (`LICENSE.txt`); the release ships them
in `assets/dictionaries/`.

| Folder | Language | wooorm folder (package version) | Licence |
|---|---|---|---|
| `en_us` | English (United States) | `en` (4.0.0) | MIT AND BSD |
| `en_gb` | English (United Kingdom) | `en-GB` (3.0.0) | MIT AND BSD |
| `en_au` | English (Australia) | `en-AU` (3.0.0) | MIT AND BSD |
| `da_dk` | Dansk | `da` (6.0.0) | GPL-2.0 OR LGPL-2.1 OR MPL-1.1 |
| `de_de` | Deutsch | `de` (3.0.0) | GPL-2.0 OR GPL-3.0 |
| `es_es` | Español (España) | `es` (4.0.0) | GPL-3.0 OR LGPL-3.0 OR MPL-1.1 |
| `fr_fr` | Français | `fr` (3.0.0) | MPL-2.0 |
| `pl_pl` | Polski | `pl` (2.0.0) | GPL-3.0 OR LGPL-3.0 OR MPL-2.0 |
| `pt_br` | Português (Brasil) | `pt` (4.0.0) | LGPL-3.0 OR MPL-2.0 |
| `ru_ru` | Русский | `ru` (3.0.0) | BSD-3-Clause |

`sl/sl.dic` is the Second Life glossary of the Legacy Viewer's dictionary
package (its words, sorted, with "Aurora" added): originally LGPL 2.1,
distributed here under GPL-3.0-or-later like the other Legacy Viewer
material (see NOTICE.md).
