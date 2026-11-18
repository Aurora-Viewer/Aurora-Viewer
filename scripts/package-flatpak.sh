#!/usr/bin/env bash
# Builds the Flatpak bundle of Aurora Viewer from a packaged Linux viewer:
# the release archive (Aurora-Viewer-<version>-linux-x64.tar.gz) or its
# extracted folder (scripts/package.ps1). Nothing is compiled: the manifest
# (packaging/flatpak/) repackages that viewer with its desktop entry, icon,
# AppStream data and sandbox permissions.
#
#   scripts/package-flatpak.sh <archive.tar.gz | folder> <version> <out.flatpak>
#
# The version (v0.9.11, 0.9.11, v0.10.0-beta.1) goes into the AppStream
# release list. Needs flatpak and flatpak-builder; the Freedesktop runtime
# and SDK of the manifest are installed from Flathub (per user) when
# missing. The bundle names Flathub as its runtime source, so
# `flatpak install --user <out.flatpak>` fetches the runtime by itself.
set -euo pipefail

if [ $# -ne 3 ]; then
    echo "usage: $0 <archive.tar.gz | folder> <version> <out.flatpak>" >&2
    exit 2
fi
input=$1
version=${2#v}
out=$3
app=org.aurora_viewer.AuroraViewer
root=$(cd "$(dirname "$0")/.." && pwd)

if ! [[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.+-]+)?$ ]]; then
    echo "invalid version: $2" >&2
    exit 2
fi

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# the manifest's sources, side by side
cp "$root/packaging/flatpak/$app.yml" "$root/packaging/flatpak/$app.desktop" "$stage/"
cp "$root/assets/branding/aurora-logo.svg" "$stage/$app.svg"
if [ -d "$input" ]; then
    [ -x "$input/aurora-viewer" ] || { echo "no aurora-viewer executable in $input" >&2; exit 1; }
    tar -C "$input" -czf "$stage/aurora-viewer-linux-x64.tar.gz" --transform 's,^\.,aurora-viewer,' .
else
    cp "$input" "$stage/aurora-viewer-linux-x64.tar.gz"
fi
# the release this bundle carries; AppStream dates are UTC
date=$(date -u +%Y-%m-%d)
release="    <release version=\"$version\" date=\"$date\">\n      <url type=\"details\">https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/v$version</url>\n    </release>"
awk -v r="$release" '{ print } /<releases>/ { gsub(/\\n/, "\n", r); print r }' \
    "$root/packaging/flatpak/$app.metainfo.xml" > "$stage/$app.metainfo.xml"

if command -v appstreamcli > /dev/null; then
    appstreamcli validate --no-net --explain "$stage/$app.metainfo.xml"
fi
if command -v desktop-file-validate > /dev/null; then
    desktop-file-validate "$stage/$app.desktop"
fi

# the runtime and SDK of the manifest, from Flathub for this user unless
# an installation already has them
runtime=$(awk -F"'" '/^runtime-version:/ { print $2 }' "$stage/$app.yml")
deps=()
if ! flatpak info "org.freedesktop.Sdk//$runtime" > /dev/null 2>&1 ||
    ! flatpak info "org.freedesktop.Platform//$runtime" > /dev/null 2>&1; then
    flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
    deps=(--user --install-deps-from=flathub)
fi
# rofiles-fuse is missing from some containers; it only speeds up a build
# that, here, copies files. The state folder sits next to the build one:
# flatpak-builder wants them on the same file system.
flatpak-builder "${deps[@]}" --disable-rofiles-fuse --force-clean --state-dir="$stage/state" \
    --repo="$stage/repo" "$stage/build" "$stage/$app.yml"
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
    "$stage/repo" "$out" "$app"
echo "bundle: $out"
