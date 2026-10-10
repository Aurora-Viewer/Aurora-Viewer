# Traduire le viewer

Chaque langue possède un catalogue Fluent dans `crates/aurora-i18n/locales/`.
Le français est dans `fr.ftl`, l'anglais dans `en.ftl`. Les fichiers sont
embarqués à la compilation ; ajouter `de.ftl`, par exemple, ajoute une langue
aux Préférences sans changer le code Rust. Le nom du fichier doit être un
identifiant de langue, comme `de` ou `pt-BR`, et `language-name` contient le
nom que ses locuteurs reconnaissent.

Le choix enregistré prime sur l'OS ; « Langue du système » suit l'OS. Les
variantes régionales utilisent le catalogue exact s'il existe, puis celui de
la langue principale. Une langue indisponible ou un message absent utilise
l'anglais. La langue et sa visibilité pour les scripts sont envoyées au
simulateur par `UpdateAgentLanguage`, après connexion, changement de région
ou modification des préférences, selon `LLAgentLanguage::update` de Firestorm.

Un nouveau texte utilise une clé explicite et les deux traductions dans la
même PR :

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

Les noms de variables doivent être identiques dans toutes les langues.
Transmettre un nombre directement permet à Fluent de choisir le pluriel :

```rust
let label = aurora_i18n::tr!("inventory-count", count = items.len());
```

```ftl
# en.ftl : 0 et 2 sont au pluriel
inventory-count = { $count ->
    [one] { $count } item
   *[other] { $count } items
    }

# fr.ftl : 0 et 1 sont au singulier
inventory-count = { $count ->
    [one] { $count } élément
   *[other] { $count } éléments
    }
```

`tr!` emprunte les libellés sans variables au cache ; un message à variables
renvoie une `String`. `Localized<T>` met en cache un tableau de libellés pour
chaque langue, afin qu'un changement s'applique aussi aux menus déjà ouverts.
Les dates sont des messages avec des composantes nommées. Pour les décimales
visibles, utiliser `number` ou `number_text`, et `integer` pour regrouper les
milliers ; les contrôles egui emploient le
même séparateur, et `parse_number` accepte celui-ci lors de la saisie.

Ne traduire que ce qu'Aurora écrit pour l'utilisateur. Les champs du protocole,
les identifiants de commandes ou de widgets, les UUID, noms de polices et
identifiants enregistrés restent stables. Les noms, descriptions et messages
écrits par les résidents ou reçus du serveur restent tels quels.

`cargo test -p aurora-i18n` vérifie la syntaxe, l'unicité des clés et la parité
des clés et variables avec l'anglais, ainsi que la sélection, le repli et les
pluriels. `scripts/check.ps1` lance aussi les tests du viewer et du réseau.

Pour comparer les langues sans modifier les préférences :

```powershell
$env:AURORA_DEMO = '1'
$env:AURORA_DEMO_OPTIONS = '1'
cargo run -p aurora-viewer -- --language en --title "Test langue-viewer"
cargo run -p aurora-viewer -- --language fr --title "Test langue-viewer"
```

Retirer `--language` pour vérifier le choix enregistré et son changement en
cours d'utilisation. Les captures utilisent les variables décrites dans
[TESTING.md](TESTING.md).
