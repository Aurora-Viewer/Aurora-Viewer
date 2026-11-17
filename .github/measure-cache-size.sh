#!/usr/bin/env bash
# TEMPORARY (measure-cache-save.yml), never merged: the size the Rust cache of
# this job would have, without saving one. Packs the same paths as
# .github/actions/rust-cache with tar and zstd, as actions/cache does, and
# counts the bytes. $1 says when (before / after the drop).
set -u
echo "== $1"
du -sh target || true
echo "package folders under target/ci/build: $(ls target/ci/build 2> /dev/null | wc -l)"
paths=(target)
for p in "$HOME/.cargo/registry/index" "$HOME/.cargo/registry/cache" "$HOME/.cargo/git/db"; do
  [ -d "$p" ] && paths+=("$p")
done
start=$(date +%s)
bytes=$(tar -cf - "${paths[@]}" 2> /dev/null | zstd -T0 --long=30 -q | wc -c)
echo "packed size ($1, tar + zstd): $((bytes / 1048576)) MB ($bytes bytes), in $(($(date +%s) - start)) s"
