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
| `categories` | Each category (its key, such as `dresses` or `body-addons`) and its words, **in the order they are tried**: the first match wins, so a "Lingerie Set" is lingerie, unless another category has a longer phrase built on the word that matched: "hair bow" (accessories) beats "hair", "dress shirt" (tops) beats "dress". Sets come last. |
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

The file names the brands creators write into item names, because that is
how items are labelled: the body makers (Reborn and eBODY, Legacy, Maitreya and
Lara, Belleza with Isis, Freya, Venus and Jake, Slink with Physique and
Hourglass, Kupra and Inithium with Erika, Signature Gianni and Geralt,
Aesthetic) and the head makers (LeLutka and EvoX, Catwa, Genus, Akeruka, Logo).
Other stores do not belong here: a store's own folder comes from its tag, and
anything store-specific beyond that is the user's store hint or creator hint in
Options. Never a resident's name or id.

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

## How to add…

| To add | Where | Example |
|---|---|---|
| A body | `bodies`: a new entry with its folder name, `strong` words (only that body uses them) and `weak` words (also ordinary words, counted only when no strong word names a body) | `{ "body": "Kira", "strong": ["kira body"], "weak": ["kira"], "hides": [] }` |
| A head platform | `categories` › `heads` for its name; the code also treats `lelutka`, `catwa`, `genus`, `evox`, `evo x` and `akeruka` as a head's name (below) | `"anotherhead"` |
| A category word | that category's `words`, at the place its priority needs: earlier categories win, and a longer phrase of another category beats the word it is built on | `"sneakers"` in `shoes`; `"platform bed"` in `furniture` beats `"platform"` in `shoes` |
| A store tag shape | `store_tag_patterns`: a pair in `enclosed` or a mark in `separators` | `["{", "}"]`, `" :: "` |
| A status word that looks like a store tag | `store_stopwords` | `"new"` |
| An unpacker word | `unpackers` | `"wear to unpack"` |
| A skin tone | `skin_tones` | `"porcelain"` |
| A Squish or Waifus spelling | `variants` | `"sqz"` |

## Rules in the code, not in the file

A few readings need more than a list of words. They live in
`crates/aurora-viewer/src/world/inventory/sorter.rs` (`classify`, `store`,
`judge_product_folder`) and are tested in `sorter_tests.rs`:

- **A permission tag is not a word of the product**: `NO MOD`, `Copy/Mod`,
  `mod/trans` are left out before anything is read (`without_permissions`),
  so `NO MOD` is never a body mod.
- **Makeup and a head's own content are the head's**: a makeup word, or an
  add-on named for a head (`Genus Addon`), never goes under a body. Beside a
  head's name only a body's own name counts (`Tattoo EVO-X [REBORN]` stays
  Reborn's), never a weak word that is also a product's (`Venus Freckles
  EvoX`).
- **A head's skin is a skin**: beside a head's name, a tone, a no-brows
  version or an overlay is a skin, unless a makeup word of its own says
  otherwise (`Tone Blush` is makeup).
- **A name that says decor is decor**: `Kitchen Decor . Glasses` is decor,
  never worn glasses (`hair decor` stays the hair's).
- **Stores**: a name written in spaced letters (`. z o r i n a .`) is one
  store, and a numbered series (`12. Store`, `#4 Store`, `(3) Store`) is that
  store, so it meets in one folder.
- **A delivery whose name says nothing** goes where three in five of its
  objects, clothing, body parts, animations and gestures read as; a mixed one
  stays.

## Global vocabulary and the user's own rules

| | Global vocabulary | The user's own |
|---|---|---|
| What | how every name is read, for everyone | Rules (My rules tab), Store hints and creator hints (Options), My folders, excluded folders |
| Where | `vocabulary.json`, built into the viewer | the user's inventory preferences, per account; Rules and hints can be shared as text (Share… / Import…) |
| Who changes it | a pull request, with the corpus test | the user, in the Auto-sort window |
| Wins | — | a rule always comes first; a store hint gives a category only to names that say nothing |

A store that a creator names oddly is never added to the vocabulary: the user
adds a store hint (`Store → Makeup`), and items that say what they are keep
their own category.

## Tests for a change

Add the names your change is for to the corpus (`corpus.json`), and for a rule
in the code a test in `sorter_tests.rs` named for what it guarantees, with
made-up store names wherever the rule does not depend on a real brand (a body
or head maker's tag is used as creators write it), and never a resident's name
or id:

```rust
/// A name that says decor is decor.
#[test]
fn decor_glasses_are_decor() {
    assert_eq!(classify("Brindle . Kitchen Decor . Glasses 8", 6, 0).category, Some(Category::Decor));
}
```

```powershell
cargo test --workspace sorter
cargo test --workspace vocabulary
```
