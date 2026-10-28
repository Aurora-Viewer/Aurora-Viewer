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

A new text uses an explicit key and both translations in the same PR:

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
```

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
