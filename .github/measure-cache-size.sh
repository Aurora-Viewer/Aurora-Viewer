#!/usr/bin/env bash
# TEMPORARY (measure-ci-deps.yml), never merged: the size the Rust cache of
# this job would have, without saving one. Drops the workspace's own outputs
# exactly as .github/actions/rust-cache/save does, then packs the same paths
# as actions/cache does (tar, zstd) and counts the bytes.
set -u
meta=$(cargo metadata --no-deps --format-version 1 --locked)
packages=$(jq -r '.packages[].name' <<< "$meta")
targets=$(jq -r '.packages[].targets[].name' <<< "$meta" | tr '-' '_' | sort -u)
for profile in target/*/; do
  [ -d "$profile.fingerprint" ] || continue
  find "$profile" -maxdepth 1 -type f -delete
  rm -rf "${profile}incremental"
  for p in $packages; do
    rm -rf "$profile.fingerprint/$p"-* "${profile}build/$p"-*
  done
  for t in $targets; do
    rm -rf "${profile}deps/$t"-* "${profile}deps/lib$t"-*
  done
done
echo "target/ without the workspace's outputs:"
du -sh target || true
paths=(target)
for p in "$HOME/.cargo/registry/index" "$HOME/.cargo/registry/cache" "$HOME/.cargo/git/db"; do
  [ -d "$p" ] && paths+=("$p")
done
if command -v zstd > /dev/null; then
  bytes=$(tar -cf - "${paths[@]}" 2> /dev/null | zstd -T0 | wc -c)
  echo "cache size (tar + zstd): $((bytes / 1048576)) MB ($bytes bytes)"
else
  echo "zstd is not installed: no packed size"
fi
