# Translating the viewer

Each language has a Fluent catalogue in `crates/aurora-i18n/locales/`.
English is in `en.ftl`, French in `fr.ftl` and Japanese in `ja.ftl`, which
are complete at every PR; the community catalogues (`de`, `es`, `it`, `nb`,
`pl`, `pt-BR`, `pt-PT`, `ru`, `tr`, `uk`, `zh-Hans`, `zh-Hant`) catch up at
each release (below). The
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
codes (`es-MX` reads `es`). `zh-CN` and `zh-SG` can use `zh-Hans`, while `zh-TW`, `zh-HK` and
`zh-MO` can use `zh-Hant`. Portuguese regions other than Brazil (`pt-AO`,
`pt-MZ`…) read `pt-PT`, `no` and Nynorsk (`nn`) read Norwegian Bokmål
(`nb`). When the language has no catalogue of its own, another region of it
comes before English (`pt-BR` would serve `pt-PT` without its catalogue);
the two Chinese scripts are never swapped this way. A missing locale or
message uses English.

The language and its visibility to scripts are sent to the simulator through
`UpdateAgentLanguage`, after login, a region change or a change to the
preferences, following the Legacy Viewer's `LLAgentLanguage::update`. The
grid gets the Legacy Viewer's code, the first part of the catalogue's
(`aurora_i18n::grid_language`: `zh-Hans` and `zh-Hant` send `zh`, `pt-BR`
sends `pt`), since scripts compare `llGetAgentLanguage` with those codes; the
currency and land XML-RPC calls, the viewer statistics, the abuse categories,
the People API, the `[LANGUAGE]` of the search, Destination Guide and
avatar welcome pages, and the web browser's language (LLUI::getLanguage)
use the same code.

A new text uses an explicit key and its English, French and Japanese versions
in the same PR, at the same place as in `en.ftl`. The other catalogues (the
community catalogues) catch up at each release (below): until then a new key
uses the current English message. The checks require valid Fluent, exact
variables on translated messages, and no stale keys in bundled catalogues.
Only English, French and Japanese require complete parity on each feature
PR. So:

- a key removed from `en.ftl` is removed from every catalogue in the same
  PR; a key renamed is renamed in every catalogue, its translation kept;
- an English message whose **meaning** changes takes a new key (the old one
  removed everywhere): the community catalogues then show English until
  their translator catches up, never a translation that says something else.
  A rewording that keeps the meaning keeps its key.

Put each catalogue's native name on a single `language-name = …` line so the
menu can display it before the catalogue is loaded. The same `.ftl` files can
be copied out of the source tree and imported through the language file
picker.

For a release, save the previous and current English catalogues and run
the delta report for each language:

```text
cargo run -q -p aurora-i18n --example catalog_delta -- previous-en.ftl crates/aurora-i18n/locales/en.ftl crates/aurora-i18n/locales/zh-Hans.ftl
```

Translate missing messages, review changed English against the existing
translation, and remove obsolete keys. The output contains English source
for review. Apply the reviewed translations to the original locale file
and run the catalogue tests before testing it in the viewer.

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

Every language addresses the user **formally**: vous in French, Sie in
German, вы in Russian, 您 in Chinese (各位 for several people, never 您们),
você in Brazilian Portuguese (the usual polite form of its software), the
です／ます form in Japanese, usted in Spanish, ви in Ukrainian, siz in Turkish
(hints in the polite `-in` imperative). Some languages are formal the way
their software is, without a pronoun: Italian gives instructions in the
impersonal infinitive ("Fare clic…") and uses Lei when it speaks to the user;
Polish uses impersonal forms ("można", "Zapisano"), never ty nor Pan/Pani;
European Portuguese uses the third person and "o seu", never você. Norwegian
Bokmål says du, as all Norwegian software does (the formal De is no longer
used). Buttons and menu items take the usual short form of the language's
software: the infinitive in French, German, Russian, Spanish, Portuguese and
Ukrainian ("Speichern", "Сохранить", "Guardar", "Зберегти"), the imperative
in Italian, Polish and Norwegian ("Salva", "Zapisz", "Lagre"), the bare stem
in Turkish ("Kaydet"). The only informal texts
are what people and objects say in the offline demo and in the built-in
gestures (the simulated residents' chat, IMs, toasts and chat logs): residents
talk to each other informally.

The community catalogues use the Second Life terms of the Legacy Viewer's
interface in their language (`xui/de`, `xui/es`, `xui/it`, `xui/pl`,
`xui/pt`, `xui/ru`, `xui/tr`, `xui/zh`), as Japanese does; Norwegian and
Ukrainian have none. The main ones, to look up before choosing another word:

- **German** (`de`): Landmarke, Parzelle, Grundbesitz (estate), Vertrag
  (covenant), Land-Info, Zuhause, Auswahlen (picks), Unterhaltung, Chat in
  der Nähe, IM, Voice, Kontakt-Set, Leute, Blockierliste, Bannliste, Foto,
  Erlebnis (experience), Einwohner (resident), Objekt (item), Eigentümer,
  Ersteller, Inventar, Einstellungen, Umgebung, Papierkorb, Fundbüro,
  Visitenkarte, Anziehen / Ausziehen, Anhang, Nehmen / Kopie nehmen, rezzen,
  Bearbeiten / Kopieren / Transferieren, Grid (the build grid is Raster).
- **Brazilian Portuguese** (`pt-BR`): Inventário, landmark (feminine),
  parcela, região, propriedade (estate), convênio (covenant), Sobre o
  terreno, Destaques (picks), Teleportar / teleporte, Casa (home), Buscar,
  chat / Chat local, IM, visual / Meus visuais (outfits), Vestir / Tirar,
  Anexar / Desanexar, notecard, prim, face, Impacto no terreno, Pegar / Pegar
  cópia, rezzar, Lixeira, Achados e perdidos, Cartão de visita, Captura,
  Lista de bloqueio, Denunciar abuso, Modificar / Copiar / Transferir.
- **Russian** (`ru`): Инвентарь, Закладка (feminine), Участок, Регион,
  Землевладение (estate), Соглашение (covenant), О земле, Житель (resident),
  Подборки (picks), Телепорт, Миникарта, Общий чат (nearby chat), ЛС (IM),
  Разговор (masculine), Внешность, Комплект (outfit), Надеть / Снять,
  Присоединить / Отсоединить, Фигура (shape), Прим, Взять / Взять копию,
  Корзина, Найденные вещи, Список заблокированных, Пожаловаться, Приключения
  (experiences), Окружающая среда, Дальность прорисовки, Изменять /
  Копировать / Передавать.
- **Chinese** (`zh-Hans` / `zh-Hant`): 库存 / 庫存, 地标 / 地標, 地块 / 地塊,
  区域 / 區域, 领地 / 領地 (estate), 瞬间传送 / 瞬間傳送, 基本体 / 元素 (prim), 垃圾桶,
  举报 / 檢舉 (report abuse), 取得 / 取得副本, 穿上 / 脫下, 卸下, 装扮 / 裝扮
  (outfit), 好友, 对话 / 對話, 屏蔽 / 封鎖 (block), 权限 / 權限, 居民.
  In zh-Hans, use 水体 for the water environment, presets and rendering controls.
  Use 水面 or 水位 for the surface or level, as appropriate. zh-Hant follows
  the Legacy Viewer's Traditional (Taiwan) terms and Taiwan software words
  (重設, 自訂, 工具列, 取得, 帳號, 套用, 清單); zh-Hans uses their Simplified
  form, with the mainland word where the Taiwan one would read oddly (脚本,
  搜索, 偏好设置, 联系人, 添加). Product names stay in Latin letters (Second
  Life, Aurora Viewer, L$), with a space between Chinese and Latin text.
- **Spanish** (`es`): Inventario, ítem (item), hito (landmark), parcela, finca
  (estate), contrato (covenant), base (home), Destacados (picks), teleportar /
  teleporte, chat local, MI (IM), Gente, Lista de bloqueados, Expulsar (eject,
  kick), Prohibir el acceso / lista de accesos prohibidos (ban, parcels, estates
  and groups alike), foto (snapshot), vestuario (outfit), Ponerme / Quitarme,
  Anexar / Quitar, anatomía (shape), rezzear, Ceder (deed), Mercado, nota
  (notecard), recurso (asset), nombre mostrado, configuración / opción
  (settings), vista subjetiva, el Flycam (free camera: cámara libre), vista
  predefinida / ajuste predefinido (preset), placa de nombre (name plate), tema
  (skin), transcripción, red (grid). Usted; buttons are infinitives (Guardar).
- **Italian** (`it`): Inventario, Biblioteca (the inventory Library; Libreria is any
  other collection), segnaposto (masculine, invariable), lotto (parcel), regione,
  proprietà immobiliare (estate), regolamento (covenant), Informazioni sul terreno,
  casa (home), Luoghi preferiti (picks), Teleport on buttons / teletrasportarsi,
  chat locale, IM (masculine), avviso di gruppo, set di contatti, Lista bloccati,
  ban / bannare (espellere is eject), Segnala abuso, foto (snapshot), abito (outfit),
  Indossa / Togli, Attacca / Stacca, attachment (masculine; allegato is a notice's),
  corpo (shape), Prendi / Prendi copia, rezzare, annotazione (notecard), gesto,
  Modifica / Copia / Trasferisci, proprietario successivo, Preferenze, nome
  visualizzato (display name), tag del nome (classic) and targhetta (Aurora's
  nameplate), marcatore (beacon), preset, fotogramma chiave, camera (feminine),
  grid (the build grid is griglia); the user is "Lei", also as the "You" label.
- **Polish** (`pl`): Szafa (inventory, feminine), przedmiot (item), prim, strona (face), mesz,
  landmark, działka (parcel), majątek (estate), umowa (covenant), O działce, miejsce startu
  (home), Miejsca (picks), czat w pobliżu, wiadomość IM, rozmowa, transkrypcja (transcript) /
  dziennik rozmów (conversation log), znajomi, członek (group member), zdjęcie (snapshot),
  strój (outfit), Załóż / Zdejmij, dodatek (attachment), rezzować, Przypisz (deed), notka,
  przygoda (experience), otoczenie, Ustawienia (Preferences), profil (preset), skórka (skin),
  tag imienia (name tag) / plakietka z imieniem (name plate), emiter / znacznik (debug /
  destination beacon), wolna kamera (flycam) / swobodna kamera (free camera), Rzeczy na
  Marketplace (listings), propozycja teleportu, Ogólne / Umiarkowane / Dla dorosłych (ratings),
  Ja (the user's own label). Impersonal address („można”, „Zapisano”, „Usunąć?”), never ty nor
  Pan/Pani; buttons in the imperative (Zapisz, Resetuj, Edytuj); quotes „…”.
- **Turkish** (`tr`): sakin (resident), envanter, öğe (item), nesne, yer imi
  (landmark), parsel, bölge, gayrimenkul (estate), Arazi Hakkında, ana konum (home),
  Favoriler (picks) / sık kullanılanlar (favorites), Işınla / ışınlanma, yakındaki
  sohbet, Aİ / anlık ileti (IM), ileti (message), sohbet (conversation), arama
  (voice call), ekran adı, grup unvanı, anlık görüntü, dış görünüm (outfit),
  aksesuar, BÜG (HUD), örgü (mesh), oluştur (rez), Değiştir / Kopyala / Aktar,
  komut dosyası, mimik (gesture), döküm (transcript), parça (track), ön ayar
  (preset), Mouselook, düşük grafikli (impostor), tema (interface skin) / ten
  (avatar skin), ad plakası / ad etiketi, Tercihler. Buttons take the bare stem
  (Kaydet, Yeniden Dene), sentences siz and "-in"; "yalnızca", not "sadece"; no
  plural after a number ("5 nesne"); labels in normal case, the viewer upper-cases
  with the dotted İ.
- **European Portuguese** (`pt-PT`): the terms of pt-BR in European spelling
  (AO90) and words: Inventário, landmark (feminine), parcela, propriedade (estate),
  convénio (covenant), Sobre o terreno, Destaques (picks), Teleportar / teleporte,
  Casa (home), Pesquisar, Chat local, IM (plural "as IM"), visual / Os meus visuais
  (outfits), Vestir / Tirar, Anexar / Desanexar, notecard, Pegar / Pegar cópia, rezzar,
  Lixo, Cartão de visita, Captura, Modificar / Copiar / Transferir, mouselook, nome de
  exibição, alcunha (nickname), sinalizador (beacon), predefinição (preset and default),
  Rodar, Expulsar, Não incomodar, recurso (asset), faixa (track), etiqueta de nome (name
  tag) / placa de nome (Aurora's name plate), balão (chat bubble), câmara livre, Realizador
  (Director), Definições; ficheiro, ecrã, guardar, rato, "A carregar…", « », «Eu», no "você".
- **Norwegian Bokmål** (`nb`): no Legacy Viewer interface; its Danish one
  (`xui/da`) is a hint only. Beholdning, gjenstand (item) / objekt, landemerke,
  parsell, eiendom (estate), avtale (covenant), Om land, beboer, Utvalgte steder
  (picks), Teleporter, minikart, Søk, chat i nærheten, IM, gruppekunngjøring,
  kontaktsett, Rapporter misbruk, foto, antrekk, Ta på / Ta av, Fest / Løsne,
  vedheng (vedlegg for a notice), På meg (worn), figur (shape), rezze, tillatelser,
  Papirkurv, Hittegods, notatkort, skript, gest, omgivelser, tale (voice),
  Innstillinger, navneskilt (name tags and plates), popup-varsel, forhåndsvalg
  (preset), skall (skin), chatutskrift (transcript) / samtalelogg, fyrlykt
  (beacon), knokkel (bone), grafikkort, Generelt / Moderat / Voksen. "du", never
  "De"; imperative buttons (Lagre, Avbryt, Bruk); sentence case, one-word
  compounds, « » quotes, › in paths, decimal comma, no space before "…".
- **Ukrainian** (`uk`), with no Legacy Viewer interface to follow: Ukrainian
  software words, never calques of the Russian ones (Налаштування, Зберегти,
  Скасувати, Вхід, Інвентар, Кошик, Знахідки). Мешканець, предмет / об'єкт,
  закладка (f.), ділянка, землеволодіння (estate), Добірки (picks), чат
  поблизу, особисте повідомлення (IM), набір контактів, комплект (outfit),
  Вдягнути / Зняти, розмістити (rez), пригода (experience), грід; іменна
  табличка for name tags and Aurora's plates, звання (group title), набір
  (preset), оформлення (skin; the Appearance page is «Вигляд»), стенограма
  (transcript), фіксація положення (Movelock), стік / тригер (gamepad); the
  Legacy Viewer is «класичний переглядач». Formal ви in lower case, buttons in
  the infinitive, plurals one / few / many / other.

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

`cargo test --workspace` (`crates/aurora-i18n/tests/catalogs.rs`) checks the syntax, the uniqueness of the keys and
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
