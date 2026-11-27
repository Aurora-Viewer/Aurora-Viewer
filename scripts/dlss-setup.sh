#!/usr/bin/env bash
# Installs NVIDIA DLSS for the viewer's DLSS option (a build with the
# `dlss` feature; docs/DLSS.md). NVIDIA's files are not ours to
# ship: this fetches them from NVIDIA's own repository for you, shows you
# NVIDIA's licence, and installs them only once you accept it.
#
#   dlss-setup.sh                 download NVIDIA's DLSS SDK and install
#   dlss-setup.sh --sdk <dir>     install from an SDK you downloaded
#   dlss-setup.sh --update        rebuild an install for this viewer's sl-ngx
#   dlss-setup.sh --remove        remove the installed files
#
# --update asks nothing: the viewer runs it at start when the folder has no
# sl-ngx of the commit below (libsl_ngx.<commit>.so), so a viewer update
# brings its sl-ngx along. Each viewer loads its own commit's, so viewers
# of different versions on one PC share the folder without rebuilding it
# in turn; the three newest are kept. It installs only when NVIDIA's
# licence is the one accepted at the install, word for word; otherwise it
# exits with status 3 and installs nothing (run this script to read the new
# licence). Its last line says what it did, or why it did not.
#
# The files go to $AURORA_DLSS_DIR, else $XDG_DATA_HOME/aurora-dlss, else
# ~/.local/share/aurora-dlss: the folder the viewer looks in at start.
#   libnvidia-ngx-dlss.so.<version>   copied from the SDK (lib/Linux_x86_64/rel)
#   libnvidia-ngx-dlssd.so.<version>  the same, for Ray Reconstruction
#   libnvidia-ngx-dlssg.so.<version>  the same, for frame generation
#   libsl_ngx.so                      sl-ngx, built here against the SDK
#   libsl_ngx.<commit>.so             the same, by its commit's first 12 digits
set -euo pipefail

REPO=https://github.com/NVIDIA/DLSS.git
# sl-ngx (MIT): NVIDIA Streamline's interface for Vulkan on NVIDIA NGX, the
# library the viewer speaks to. Fetched at this commit, never a branch.
SL_NGX_REPO=https://github.com/Aurora-Viewer/sl-ngx.git
SL_NGX_COMMIT=5b0dd82fada40c01f8c99958184eb411be600326

die() { printf 'dlss-setup: %s\n' "$*" >&2; exit 1; }

dest=${AURORA_DLSS_DIR:-}
if [[ -z $dest ]]; then
    dest=${XDG_DATA_HOME:-}
    [[ -n $dest ]] || dest=$HOME/.local/share
    dest=$dest/aurora-dlss
fi

# The licence goes in under its own name, so a LICENSE.txt of the user's in
# a custom AURORA_DLSS_DIR is left alone; a LICENSE.txt the first version of
# this script wrote (NVIDIA's, by its first line) is cleared.
LICENCE_NAME=NVIDIA-DLSS-LICENSE.txt
VERSIONED=libsl_ngx.${SL_NGX_COMMIT:0:12}.so
old_files() {
    [[ -f $dest/LICENSE.txt ]] && [[ $(head -n 1 "$dest/LICENSE.txt") == "NVIDIA RTX SDKs LICENSE"* ]] \
        && rm -f "$dest/LICENSE.txt"
    # what an earlier version of this script built in sl-ngx's place
    rm -f "$dest/libnvsdk_ngx.so"
    return 0
}

# AURORA_DLSS_DIR anywhere inside an SDK checkout, at any depth (links
# followed): removing would delete the SDK's own files, and installing
# clears the other libnvidia-ngx-dlss.so.* there, the SDK's own versions
# when it is the SDK's rel folder. Judged by the shape:
# the folder or one above it holds the SDK's lib/Linux_x86_64 and its
# LICENSE.txt, or the path runs through a lib/Linux_x86_64 folder.
in_sdk() {
    local d
    d=$(realpath -m "$1")
    case $d/ in
        */lib/Linux_x86_64/*) return 0 ;;
    esac
    [[ ! -d $d/lib/Linux_x86_64 ]] || return 0
    while [[ $d != / && -n $d ]]; do
        [[ -d $d/lib/Linux_x86_64 && -f $d/LICENSE.txt ]] && return 0
        d=${d%/*}
    done
    return 1
}

sdk=
update=
case ${1:-} in
    "") ;;
    --sdk) [[ -n ${2:-} ]] || die "--sdk needs the SDK's folder"; sdk=$2 ;;
    --update)
        update=1
        [[ -f $dest/libsl_ngx.so && -f $dest/$LICENCE_NAME ]] \
            || die "no install in $dest to update: run this script without --update"
        if [[ -f $dest/$VERSIONED ]]; then
            echo "sl-ngx in $dest is already ${SL_NGX_COMMIT:0:12}: nothing to do."
            exit 0
        fi
        # (two viewers starting at once: one update)
        exec 9>"$dest/.update.lock"
        command -v flock >/dev/null && { flock -n 9 || die "another update of $dest is running"; }
        ;;
    --remove)
        ! in_sdk "$dest" \
            || die "AURORA_DLSS_DIR is inside NVIDIA's DLSS SDK ($dest): nothing removed, choose another"
        rm -f "$dest"/libnvidia-ngx-dlss.so.* "$dest"/libnvidia-ngx-dlssd.so.* "$dest"/libnvidia-ngx-dlssg.so.* \
            "$dest"/libsl_ngx.so "$dest/$LICENCE_NAME" "$dest"/libsl_ngx.*.so "$dest/.update.lock"
        old_files
        rmdir "$dest" 2>/dev/null || true
        echo "Removed NVIDIA DLSS from $dest. The viewer offers FSR alone from its next start."
        exit 0 ;;
    -h|--help) sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
esac

command -v git >/dev/null || die "git is needed to download sl-ngx (Arch: pacman -S git)"
command -v make >/dev/null || die "make is needed to build sl-ngx (Arch: pacman -S make)"
command -v g++ >/dev/null || die "g++ is needed to build sl-ngx (Arch: pacman -S gcc)"
printf '#include <vulkan/vulkan.h>\n' | g++ -x c++ -fsyntax-only - 2>/dev/null \
    || die "the Vulkan headers are needed to build sl-ngx (Arch: pacman -S vulkan-headers; Debian, Ubuntu: apt install libvulkan-dev)"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [[ -z $sdk ]]; then
    echo "Downloading NVIDIA's DLSS SDK from $REPO (its licence, headers and Linux files only, about 60 MB)..."
    sdk=$work/DLSS
    git clone --quiet --depth 1 --filter=blob:none --no-checkout "$REPO" "$sdk"
    git -C "$sdk" sparse-checkout set --no-cone \
        /LICENSE.txt /include/ /lib/Linux_x86_64/libnvsdk_ngx.a '/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.*' \
        '/lib/Linux_x86_64/rel/libnvidia-ngx-dlssd.so.*' '/lib/Linux_x86_64/rel/libnvidia-ngx-dlssg.so.*'
    git -C "$sdk" checkout --quiet
fi

licence=$sdk/LICENSE.txt
snippets=("$sdk"/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.*)
[[ -f $licence ]] || die "no LICENSE.txt in $sdk: is it NVIDIA's DLSS SDK?"
[[ -f $sdk/lib/Linux_x86_64/libnvsdk_ngx.a ]] || die "no lib/Linux_x86_64/libnvsdk_ngx.a in $sdk"
[[ -f $sdk/include/nvsdk_ngx_vk.h ]] || die "no include/nvsdk_ngx_vk.h in $sdk"
[[ -f ${snippets[0]} ]] || die "no lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.* in $sdk"
# the newest version, by number (310.10 after 310.9)
snippet=$(printf '%s\n' "${snippets[@]}" | sort -V | tail -n 1)
# Ray Reconstruction's library: optional, an older SDK has none
rr_snippets=("$sdk"/lib/Linux_x86_64/rel/libnvidia-ngx-dlssd.so.*)
rr_snippet=
[[ -f ${rr_snippets[0]} ]] && rr_snippet=$(printf '%s\n' "${rr_snippets[@]}" | sort -V | tail -n 1)
# frame generation's library: optional too
fg_snippets=("$sdk"/lib/Linux_x86_64/rel/libnvidia-ngx-dlssg.so.*)
fg_snippet=
[[ -f ${fg_snippets[0]} ]] && fg_snippet=$(printf '%s\n' "${fg_snippets[@]}" | sort -V | tail -n 1)
# AURORA_DLSS_DIR inside an SDK (in_sdk), or the selected SDK's own folder
# whatever its shape
dest_real=$(realpath -m "$dest")
! in_sdk "$dest" || die "AURORA_DLSS_DIR is inside NVIDIA's DLSS SDK ($dest): choose another"
for d in "$sdk" "$sdk/lib/Linux_x86_64" "$sdk/lib/Linux_x86_64/rel"; do
    [[ $dest_real != "$(realpath -m "$d")" ]] \
        || die "AURORA_DLSS_DIR is the SDK's own folder $d: choose another"
done

if [[ -n $update ]]; then
    # accepted at the install, word for word, or nothing is installed
    if ! cmp -s "$licence" "$dest/$LICENCE_NAME"; then
        printf 'dlss-setup: %s\n' "NVIDIA's licence for the DLSS SDK has changed: run dlss-setup.sh to read it; nothing installed" >&2
        exit 3
    fi
else
    echo
    echo "NVIDIA's DLSS SDK is under NVIDIA's licence, not the viewer's. Read it:"
    echo
    ${PAGER:-less} "$licence" || cat "$licence"
    echo
    read -r -p "Do you accept NVIDIA's licence for the DLSS SDK? Type yes to install: " answer || answer=
    [[ $answer == yes ]] || die "not accepted: nothing installed"
fi

echo "Downloading sl-ngx ${SL_NGX_COMMIT:0:12} from $SL_NGX_REPO and building it against the SDK..."
src=$work/sl-ngx
git init --quiet "$src"
git -C "$src" fetch --quiet --depth 1 "$SL_NGX_REPO" "$SL_NGX_COMMIT"
git -C "$src" checkout --quiet FETCH_HEAD
[[ $(git -C "$src" rev-parse HEAD) == "$SL_NGX_COMMIT" ]] || die "sl-ngx is not at $SL_NGX_COMMIT"
make --quiet -C "$src" DLSS_SDK="$(realpath "$sdk")" libsl_ngx.so >/dev/null \
    || die "sl-ngx did not build (its messages are above)"

mkdir -p "$dest"
# Staged beside the install, on its filesystem: each file copied under a
# hidden temporary name and compared with its source, then renamed over
# its place (one step on one filesystem). The older runtimes go only once
# the new ones are in: a copy that fails (a full disk) leaves the working
# install as it was, and the temporary files go with the script's exit.
new_snippet=$(basename "$snippet")
new_rr=
[[ -z $rr_snippet ]] || new_rr=$(basename "$rr_snippet")
new_fg=
[[ -z $fg_snippet ]] || new_fg=$(basename "$fg_snippet")
staged=()
trap 'rm -rf "$work"; rm -f "${staged[@]}"' EXIT
stage() { # <source> <name> <mode>
    local tmp="$dest/.$2.aurora-new"
    staged+=("$tmp")
    install -m "$3" "$1" "$tmp" || die "cannot copy $2 to $dest: the install there is unchanged"
    cmp -s "$1" "$tmp" || die "the copy of $2 in $dest differs from its source: the install there is unchanged"
}
stage "$snippet" "$new_snippet" 755
# (--update leaves libsl_ngx.so to the viewers older than this script,
# which know no other name)
stage "$src/libsl_ngx.so" "$VERSIONED" 755
names=("$new_snippet" "$VERSIONED" "$LICENCE_NAME")
if [[ -z $update ]]; then
    stage "$src/libsl_ngx.so" libsl_ngx.so 755
    names+=(libsl_ngx.so)
fi
stage "$licence" "$LICENCE_NAME" 644
if [[ -n $new_rr ]]; then
    stage "$rr_snippet" "$new_rr" 755
    names+=("$new_rr")
fi
if [[ -n $new_fg ]]; then
    stage "$fg_snippet" "$new_fg" 755
    names+=("$new_fg")
fi
for name in "${names[@]}"; do
    mv -f "$dest/.$name.aurora-new" "$dest/$name"
done
staged=()
shopt -s nullglob
for old in "$dest"/libnvidia-ngx-dlss.so.*; do
    [[ $(basename "$old") == "$new_snippet" ]] || rm -f "$old"
done
# (Ray Reconstruction's too: all of them when this SDK has none)
for old in "$dest"/libnvidia-ngx-dlssd.so.*; do
    [[ $(basename "$old") == "$new_rr" ]] || rm -f "$old"
done
# (frame generation's the same way)
for old in "$dest"/libnvidia-ngx-dlssg.so.*; do
    [[ $(basename "$old") == "$new_fg" ]] || rm -f "$old"
done
# the three newest sl-ngx by commit, this one among them (a viewer still
# running keeps the one it loaded)
mapfile -t versioned < <(ls -1t "$dest"/libsl_ngx.*.so)
for old in "${versioned[@]:3}"; do
    [[ $(basename "$old") == "$VERSIONED" ]] || rm -f "$old"
done
shopt -u nullglob
old_files

if [[ -n $update ]]; then
    echo "Updated sl-ngx in $dest to ${SL_NGX_COMMIT:0:12}, with $(basename "$snippet"): the viewer uses it from its next start."
    exit 0
fi
echo
echo "Installed $(basename "$snippet")${rr_snippet:+, $(basename "$rr_snippet")}${fg_snippet:+, $(basename "$fg_snippet")} and libsl_ngx.so in $dest."
[[ -n $rr_snippet ]] || echo "This SDK has no Ray Reconstruction library (libnvidia-ngx-dlssd.so.*): Ray Reconstruction uses the driver's own, if it has one."
[[ -n $fg_snippet ]] || echo "This SDK has no frame generation library (libnvidia-ngx-dlssg.so.*): frame generation stays off."
echo "Restart the viewer, then choose NVIDIA DLSS under Preferences > Graphics > Anti-aliasing."
echo "NVIDIA graphics cards only."
