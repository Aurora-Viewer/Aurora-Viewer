# Aurora Viewer Flatpak

Builds [Aurora Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer) as a
Flatpak (`org.aurora_viewer.AuroraViewer`).

Files:

- `org.aurora_viewer.AuroraViewer.yml` — the Flatpak manifest.
- `cargo-sources.json` — vendored crate sources for the offline build,
  generated from `Cargo.lock` with `flatpak-cargo-generator.py`.
- `org.aurora_viewer.AuroraViewer.desktop` — desktop entry, adapted from
  `packaging/linux/aurora-viewer.desktop` (Flatpak icon name).
- `org.aurora_viewer.AuroraViewer.metainfo.xml` — AppStream metadata.

## Prerequisites

```sh
flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install flathub org.freedesktop.Platform//25.08 org.freedesktop.Sdk//25.08
flatpak install flathub org.freedesktop.Sdk.Extension.rust-stable//25.08
```

## Build and install

From the repository root:

```sh
flatpak-builder --force-clean --install --user builddir packaging/linux/flatpak/org.aurora_viewer.AuroraViewer.yml
```

Run with `flatpak run org.aurora_viewer.AuroraViewer`.

The manifest builds the checkout as it is through its `dir` source. For a
faster build, use a clean checkout or worktree.

## After Cargo.lock changes

The offline build only sees the crates vendored in `cargo-sources.json`.
Whenever `Cargo.lock` changes, regenerate it from the repo root:

```sh
python3 flatpak-cargo-generator.py Cargo.lock -o packaging/linux/flatpak/cargo-sources.json
```

(`flatpak-cargo-generator.py` is in
[flatpak-builder-tools](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo).)
