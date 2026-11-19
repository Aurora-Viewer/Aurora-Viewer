# Translating the viewer

Each language has a Fluent catalogue in `crates/aurora-i18n/locales/`.
English is in `en.ftl`, French in `fr.ftl` and Japanese in `ja.ftl`. The
files are embedded at build time; adding `de.ftl`, for example, adds a
language to the Preferences without changing the Rust code. The file name
must be a language identifier, such as `de` or `pt-BR`, and `language-name`
holds the name that its speakers recognise.

The embedded fonts only cover European scripts. Other scripts, Japanese
included, are drawn by the system fallback fonts (Meiryo UI or Yu Gothic UI
for Japanese on Windows, see `ui/fonts/fallback.rs`), found before the first
frame. egui breaks Japanese and Chinese lines between characters. A date is
a message with named components (`date-day-month`), never an order written
in the code. Japanese has no plural: a plural select of the English becomes
plain text (`{ $count }件`), while a select on anything else keeps its
variants. Japanese uses the official Second Life terms of the Legacy
Viewer's `xui/ja` interface (インベントリ, 区画, リージョン, 初期設定…).

"System language" is the default and follows the OS. An explicit saved
choice takes precedence. Preferences shows the system match first among
bundled languages, then common choices and the rest by language code. Each
entry uses its native name and the exact catalogue code, such as `English
(en)` or `日本語 (ja)`.

Regional variants prefer the exact catalogue and then try less specific
codes. `zh-CN` and `zh-SG` can use `zh-Hans`, while `zh-TW`, `zh-HK` and
`zh-MO` can use `zh-Hant`. A missing locale or message uses English. The
language and its visibility to scripts are sent to the
simulator through `UpdateAgentLanguage`, after login, a region change or a
change to the preferences, following the Legacy Viewer's `LLAgentLanguage::update`.

A new text uses an explicit key and its English, French and Japanese versions
in the same PR, at the same place as in `en.ftl`. Community catalogues can
catch up on a separate release schedule. New untranslated keys use the
current English message until the community catalogue gains a translation.
The checks require valid Fluent, exact variables on translated messages,
and no stale keys in bundled catalogues. Only English, French and Japanese
require complete parity on each feature PR. Put each catalogue's native name
on a single `language-name = …` line so the menu can display it before the
catalogue is loaded. The same `.ftl` files can be copied out of the source
tree and imported through the language file picker.

```rust
ui.label(aurora_i18n::tr!("inventory-empty"));
let message = aurora_i18n::tr!("resident-online", name = name.as_str());
```

```ftl
# en.ftl
inventory-empty = Inventory is empty.
resident-online = { $name } is online.

# fr.ftl
inventory-empty = L'inventaire est vide.
resident-online = { $name } est en ligne.

# ja.ftl
inventory-empty = インベントリは空です。
resident-online = { $name }さんがオンラインになりました。
```

Variable names must be identical in every language.
Passing a number directly lets Fluent choose the plural:

```rust
let label = aurora_i18n::tr!("inventory-count", count = items.len());
```

```ftl
# en.ftl: 0 and 2 are plural
inventory-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
    }

# fr.ftl: 0 and 1 are singular
inventory-count = { $count ->
    [one] { $count } élément
   *[other] { $count } éléments
    }

# ja.ftl: no plural, plain text
inventory-count = { $count }個のアイテム
```

Japanese, beyond the plural:

- Buttons, menu items, titles and short labels are concise nouns or verb
  stems (保存, 削除, フレンドに追加), without です／ます nor a final 。;
  sentences, hints and errors use the polite です／ます form.
- Japanese punctuation is full width (、。「」（）！？：); no space between
  Japanese text and a placeable, a number or a unit (`{ $seconds }秒`,
  `{ $distance }ｍ`).
- Abbreviations, units and digits written in the Japanese text are full
  width (ＵＵＩＤ, ＬＳＬ, ＨＵＤ, ＵＲＬ, ３Ｄ, ｍ, ＭＢ, ％, １つ):
  half-width capitals look squashed next to kana and kanji, and the Legacy
  Viewer's Japanese interface writes them the same way. The value of a
  placeable is formatted by the code and stays as it is, and so does what
  the user types or compares with another text: key names (Ctrl, WASD), chat
  commands, file names and extensions, the identifiers of the Dev Toolbox
  tools (J2, L1).
- Terms: アバター, インベントリ, フレンド, グループ, 区画 (parcel),
  リージョン, 不動産 (estate), オブジェクト, 装着 (attach), アウトフィット,
  容姿 (appearance), 初期設定 (Preferences), 設定 (Aurora's own settings),
  近くのチャット, テレポート, ランドマーク, スクリプト, テクスチャ, フォルダ,
  ごみ箱; look an existing one up in `ja.ftl` before choosing a new one.
  Second Life, L$ and product names (Blender) stay in Latin letters. The
  viewers are Auroraビューア and レガシービューア: "Legacy" is レガシー, as in
  the option labels the Legacy Viewer's Japanese interface already
  translates (昼間（レガシー）).
- A label quoted in a hint (「…」) is the exact text of that label in
  `ja.ftl`.
- A continuation line of a multi-line message cannot start with `*`, `[`
  or `.`: Fluent reads it as a variant. Reword it (Japanese often starts
  with the Markdown bold of the English, `**…**`).

`tr!` borrows labels without variables from the cache; a message with
variables returns a `String`. `Localized<T>` caches an array of labels for
each language, so that a change also applies to the menus already open.
Dates are messages with named components. For visible decimals, use
`number` or `number_text`, and `integer` to group thousands; the egui
controls use the same separator, and `parse_number` accepts it on input.

Translate only what Aurora writes for the user. Protocol fields, command or
widget identifiers, UUIDs, font names and saved identifiers stay stable.
Names, descriptions and messages written by residents or received from the
server stay as they are.

`cargo test -p aurora-i18n` checks the syntax, the uniqueness of the keys and
the parity of the keys and variables with English, as well as selection,
fallback and plurals. `scripts/check.ps1` also runs the viewer and network
tests.

## Trying a translation without building

Preferences › Language and updates › "Use a language file (.ftl)" loads a
catalogue file of your own instead of the list's language, without building
the viewer: start from a copy of `en.ftl`, name it after its language (`de.ftl`,
`pt-BR.ftl`, which gives the plural rules), translate, choose it with
Browse… and start the viewer again. The file is read at startup only. The
texts it lacks are shown in English, and the line under the button counts
the texts it holds and the entries Fluent could not read. The file must be
UTF-8 (Notepad's byte order mark is accepted). To add the language to the
viewer for everyone, put the file in `crates/aurora-i18n/locales/` as
described above.

For an in-progress language, keep only translated messages in the file.
After a viewer update, new messages absent from the file use the English
text shipped with that build. Invalid Fluent entries are skipped. If a
translated message cannot be formatted (for example, it still uses a
variable removed from the call), the viewer tries English with the same
arguments. If English also has no usable message, the key is displayed.
When updating a catalogue, review existing translations against changes to
the English text, since an older entry can still render after its meaning
changes.

To compare the languages without changing the preferences:

```powershell
$env:AURORA_DEMO = '1'
$env:AURORA_DEMO_OPTIONS = '1'
cargo run -p aurora-viewer -- --language en --title "Test langue-viewer"
cargo run -p aurora-viewer -- --language fr --title "Test langue-viewer"
cargo run -p aurora-viewer -- --language ja --title "Test langue-viewer"
```

Remove `--language` to check the saved choice and its change while the
viewer is running. Screenshots use the variables described in
[TESTING.md](TESTING.md).
