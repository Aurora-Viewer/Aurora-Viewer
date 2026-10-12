# V-HACD 4.1

`VHACD.h` et `LICENSE` proviennent du tag public `v4.1.0`, révision
`22ec20a7f8ea221ab600df869369b9c6a258cb10` du
[dépôt V-HACD](https://github.com/kmammou/v-hacd/tree/22ec20a7f8ea221ab600df869369b9c6a258cb10).
SHA-256 du fichier d’origine :
`02314c575909d656dc67d6e3c38cefc49c4403e8ca1dcb0ab4b05800cee61e1c`.

La version est figée pour garder les modes Flood, Surface et Raycast et les
paramètres d’analyse employés par Firestorm. Le header est conservé sans
modification sous sa licence BSD-3-Clause. `bridge.cpp` est l’adaptateur Aurora :
propriété des allocations, exceptions contenues, annulation et progression.
Les modèles sont traités en parallèle par les workers Rust ; chaque instance
native fonctionne de façon synchrone pour éviter la multiplication des threads.
