#!/usr/bin/env bash
# Removes the outputs of the workspace's own packages from target/ before
# save/action.yml saves the Rust cache. Aurora's crates are rebuilt by every
# run that restores the cache: the checkout gives their sources a newer time
# than the cached outputs, so Cargo finds every one of them stale, even the
# crates a change did not touch. Kept, they only make the cache larger.
#
# Cargo's build folder, under each target/<profile>/:
# - since Rust 1.100: build/<package>/<hash>/ (fingerprint/, out/, run/), one
#   folder per package for all of its units: library, tests, examples, build
#   script and its run;
# - before it: .fingerprint/<package>-<hash>, build/<package>-<hash> and
#   deps/<target>-<hash>.* (targets underscored), still handled in case an
#   older toolchain writes it again.
# In both, the binaries, libraries and .d files copied to the top of the
# profile, examples/ (only the workspace's examples are ever built) and
# incremental/ go too. The dependencies' folders, the outputs of their build
# scripts and the registry stay.
#
# The times say which command a hung save stopped in (it hung for hours once
# on Windows, see ci.yml).
set -eu

stamp() { echo "$(date -u +%H:%M:%S) $*"; }

stamp "cargo metadata"
meta=$(cargo metadata --no-deps --format-version 1 --locked)
# jq on Windows ends its lines with CR LF: a name that kept the CR would
# match no folder
packages=$(jq -r '.packages[].name' <<< "$meta" | tr -d '\r')
targets=$(jq -r '.packages[].targets[].name' <<< "$meta" | tr -d '\r' | tr '-' '_' | sort -u)

stamp "dropping the workspace's outputs"
# package folders found and removed, in both layouts
found=0
for profile in target/*/; do
  [ -d "${profile}build" ] || [ -d "${profile}.fingerprint" ] || continue
  stamp "profile $profile"
  find "$profile" -maxdepth 1 -type f -delete
  rm -rf "${profile}incremental" "${profile}examples"
  for p in $packages; do
    if [ -d "${profile}build/$p" ]; then
      rm -rf "${profile}build/$p"
      found=$((found + 1))
    fi
  done
  if [ -d "${profile}.fingerprint" ]; then
    for p in $packages; do
      for f in "${profile}.fingerprint/$p"-*; do
        [ -e "$f" ] && found=$((found + 1))
      done
      rm -rf "${profile}.fingerprint/$p"-* "${profile}build/$p"-*
    done
    for t in $targets; do
      rm -rf "${profile}deps/$t"-* "${profile}deps/lib$t"-*
    done
  fi
done
stamp "$found package folders dropped"
# the save runs after a build of the workspace: finding none of its folders
# means Cargo laid them out another way, and the cache keeps them
if [ "$found" -eq 0 ]; then
  echo "::warning::no output of the workspace's packages found under target/: the cache keeps them. Has Cargo's build folder changed? See .github/actions/rust-cache/save/drop-workspace-outputs.sh"
fi

stamp "size"
du -sh target || true
stamp "done"
