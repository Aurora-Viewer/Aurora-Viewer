# Auto-sort vocabulary

Auto-sort reads an item's name the way creators write it: a body tag
(`[REBORN]`, `Lara X`, `LegacyM+AT`), a category word (`Dress`, `Boots`,
`Sofa`), an unpacker (`Rez to unpack`), a Squish or Waifus version, and a
store tag (`[Store] +`, `*Store*`, `Store - …`). These words are not in the
code: they live in one file,
[`crates/aurora-viewer/assets/auto-sort/vocabulary.json`](../crates/aurora-viewer/assets/auto-sort/vocabulary.json),
built into the viewer. Anyone can add a word there without reading Rust.

## The file

| Section | What it holds |
|---|---|
| `bodies` | Each body (`body`, the folder name), its `strong` words, its `weak` words (only counted when no strong word names a body: `lara` or `juicy` are also ordinary words) and the bodies it `hides` (`eBODY Reborn` is Reborn, not eBODY). |
| `categories` | Each category (its key, such as `dresses` or `body-addons`) and its words, **in the order they are tried**: the first match wins, so a "Lingerie Set" is lingerie. Sets come last. |
| `skin_tones` | Tones creators name body skins with (`porcelain`): a body name with a tone is a skin. |
| `unpackers` | Words of a box that gives the real items when worn or rezzed. |
| `variants` | `squish` (`sq`, `squeeze`…) and `waifus` words: these versions get their own folder below the category. |
| `store_tag_patterns` | `enclosed`: the pairs a store is written between (`[` `]`, `*` `*`…); `separators`: what follows a store at the start of a name (` - `, ` // `, `: `…). |
| `store_stopwords` | Words of a status written like a store tag (`mesh`, `demo`, `new`…): a tag made only of them (`(Mesh)`, `[DEMO]`) is not a store. |

### Writing words

A name is read as lowercase words: everything that is not a letter or a digit
separates words. A word or phrase of the file is written the same way:
lowercase, single spaces, no punctuation (`evo x`, `add on`, `wear to
unpack`). An apostrophe joins its word: `women's` is read `womens` (an
item name is read both as `women`, `s` and as `womens`, so rules written
either way still match), and a rule's or a store tag's words are read joined
only. A word written otherwise could never match, so the viewer refuses
the file and says which word, for instance `"Tattoo" is read as "tattoo"`.
An unknown section, an unknown category or a body that hides a body that does
not exist are refused too.

Never put a real resident's name or id in the file. Store names in general do
not belong here either: a store is read from its tag, and users who want a
store filed somewhere add a store hint or a creator hint in Options.

## Checking a change

The golden corpus,
[`assets/auto-sort/corpus.json`](../crates/aurora-viewer/assets/auto-sort/corpus.json),
holds names as creators write them and where each one should go, with a folder
per store and no rules. `{dresses}` is a category's folder (its key) and
`{stores}` the Stores folder; `null` means the name says nothing and the entry
stays.

```powershell
cargo test --workspace vocabulary
```

On a difference, the test lists every name that moved, what was expected and
what it got. Either the change is wrong (fix the vocabulary) or the new result
is better (update the corpus, and say why in the PR). Add the names your change
is for to the corpus, so that nobody breaks them later.

To see it in the viewer, in a development build:

```powershell
$env:AURORA_DEMO = "1"
$env:AURORA_DEMO_INVENTORY = "sorter-vocab"
$env:AURORA_SORTER_VOCAB = "C:\path\to\my-vocabulary.json"   # optional
cargo run -p aurora-viewer -- --title "Test vocabulary"
```

The Vocabulary check window lists every corpus name, expected against result,
differences first in the warning colour. `AURORA_SORTER_VOCAB` makes a
development build read another file instead of the built-in one, so a change
can be tried without rebuilding; a file that does not read is ignored, with
the reason in the log. A published (release) build always uses the built-in
vocabulary.
