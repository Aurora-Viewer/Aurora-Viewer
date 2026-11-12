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

The saved choice takes precedence over the OS; "System language" follows the
OS. Regional variants use the exact catalogue if it exists, then the one of
the main language. An unavailable language or a missing message uses
English. The language and its visibility to scripts are sent to the
simulator through `UpdateAgentLanguage`, after login, a region change or a
change to the preferences, following the Legacy Viewer's `LLAgentLanguage::update`.

A new text uses an explicit key and its version in every catalogue
(English, French and Japanese) in the same PR, at the same place as in
`en.ftl`. A key missing from one catalogue fails the parity test, in
`scripts/check.ps1` and in the merge queue.

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
- Japanese punctuation is full width (、。「」（）！？：), the colon of a
  label included (`オブジェクト：{ $object }`); no space between Japanese
  text and a placeable, a number or a unit (`{ $seconds }秒`,
  `{ $distance }ｍ`, `{ $key }キー`, `{ $time }㎳`). A name inserted in a
  sentence goes between 「」 (`「{ $experience }」という体験`).
- Dimensions use ✕ without spaces (`{ $x }✕{ $y }✕{ $z }ｍ`); a gap
  inside the Japanese text is a full-width space (`{ $first }…　{ $last }`).
- Abbreviations, units and digits written in the Japanese text are full
  width (ＵＵＩＤ, ＬＳＬ, ＨＵＤ, ＵＲＬ, ＶＲ, ＰＢＲ, ３Ｄ, ｍ, ＭＢ, ％,
  １つ, １度):
  half-width capitals look squashed next to kana and kanji, and the Legacy
  Viewer's Japanese interface writes them the same way. The value of a
  placeable is formatted by the code and stays as it is, and so does what
  the user types or compares with another text: key names (Ctrl, WASD), chat
  commands, file names and extensions, the identifiers of the Dev Toolbox
  tools (J2, L1).
- Terms: アバター, インベントリ, フレンド, グループ, 人物 (People, the
  window or tab; never 人々), 区画 (parcel),
  リージョン, 不動産 (estate), オブジェクト, 装着 (attach), 装着物
  (attachment), アウトフィット, 外観 (appearance: an avatar's, which may
  be a robot or an animal, and the interface's; never 容姿), 所有者
  (owner),
  初期設定 (Preferences only), デフォルト (a default value or setting:
  デフォルトではオフ, デフォルトに戻す; never 既定, as residents already say
  デフォルトアバター), 設定 (Aurora's
  own settings), 近くのチャット,
  テレポート, ランドマーク, スクリプト, テクスチャ, フォルダ, ごみ箱,
  複製 (copy, the permission: 複製不可, 複製と譲渡の権限; and Duplicate),
  コピー (copy, anywhere else: the clipboard, a copy of an item), モニター
  (a display),
  ビルド (build: the tools, the window, the permission), 建物 (a building
  in the world), 体験 (experience), 自然環境 (environment, EEP), 露光
  (exposure), 光源 (a light: a prim's Light, the local and worn lights,
  and the lighting of a photo or a portrait), 射影テクスチャ (a light's
  projected texture), メタル (metal), ＨＤ画像 (the paid HD picture of an
  outfit), ベイク (bake) and 再ベイク (rebake), 樹木 (a tree), 水面 (water:
  the water settings, a water preset, swimming in it; underwater fog is
  水中のフォグ), 飛行カメラ (flycam);
  look an existing one up in `ja.ftl` before choosing a new one. Never
  持ち物, エクスペリエンス, アタッチ, オーナー, 露出, フォルダー, モニタ,
  ベーク, リベーク, フライカム nor ライト or ライティング for a light: they are the
  variants these replace (ハイライト,
  スポットライト and the residents' フェイスライト stay). 建築 stays only in the
  residents' own words of the demo (今夜は広場を建築するよ), never for the
  tools.
- A material map keeps its Japanese name with the usual katakana between
  brackets on its label: 拡散（ディフューズ）, 法線（ノーマル）,
  鏡面反射（スペキュラ）, 粗さ（ラフネス）, ベースカラー（アルベド）. A
  sentence or a compound uses the Japanese name alone (法線＋鏡面反射).
- The maturity ratings General, Moderate and Adult stay in Latin letters,
  as in the Legacy Viewer (Moderate向けイベント, Adult指定).
- Second Life, L$ and product names (Blender) stay in Latin letters. The
  viewers are Auroraビューア and レガシービューア: "Legacy" is レガシー, as in
  the option labels the Legacy Viewer's Japanese interface already
  translates (昼間（レガシー）).
- LiveMesh is ライブメッシュ on screen, to read like the ライブオブジェクト and
  ライブマテリアル of the same feature. The Blender add-on keeps its real
  name where the user has to find it: Aurora LiveMesh.
- A label quoted in a hint (「…」) is the exact text of that label in
  `ja.ftl`.
- A menu or settings path uses the full-width ＞ with each step between 「」
  and no space (「初期設定」＞「Aurora」＞「通知」), as the Legacy Viewer's
  Japanese interface does; a step can carry a word after its bracket
  (「ワールド」メニュー＞「…」, 「土地情報」＞「一般」タブ). Never ›, > or → for a
  path. The same ＞ ends a text link that opens more (すべて読む＞) and
  separates the levels of a category (年齢＞年齢偽証), without brackets. →
  stays for a change or an order (あなた→次の所有者, 名前：A→Z). The comments
  of the catalogues stay in English and keep ›.
- A continuation line of a multi-line message cannot start with `*`, `[`
  or `.`: Fluent reads it as a variant. Reword it (Japanese often starts
  with the Markdown bold of the English, `**…**`). Its indent is ASCII
  spaces, never the full-width space: Fluent does not take 　 as an indent
  and reads the line as a broken entry.

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
