#!/usr/bin/env bash
# Installs NVIDIA DLSS for the viewer's DLSS option (a build with the
# `dlss` feature; docs/DLSS.md). NVIDIA's files are not ours to
# ship: this fetches them from NVIDIA's own repository for you, shows you
# NVIDIA's licence, and installs them only once you accept it.
#
#   dlss-setup.sh                 download NVIDIA's DLSS SDK and install
#   dlss-setup.sh --sdk <dir>     install from an SDK you downloaded
#   dlss-setup.sh --remove        remove the installed files
#
# The files go to $AURORA_DLSS_DIR, else $XDG_DATA_HOME/aurora-dlss, else
# ~/.local/share/aurora-dlss: the folder the viewer looks in at start.
#   libnvidia-ngx-dlss.so.<version>   copied from the SDK (lib/Linux_x86_64/rel)
#   libsl_ngx.so                      sl-ngx, built here against the SDK
set -euo pipefail

REPO=https://github.com/NVIDIA/DLSS.git
# sl-ngx (MIT): NVIDIA Streamline's interface for Vulkan on NVIDIA NGX, the
# library the viewer speaks to. Fetched at this commit, never a branch.
SL_NGX_REPO=https://github.com/Aurora-Viewer/sl-ngx.git
SL_NGX_COMMIT=4016e7cbd5d05dd07b5c0d7743d76b1a3d248380

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
case ${1:-} in
    "") ;;
    --sdk) [[ -n ${2:-} ]] || die "--sdk needs the SDK's folder"; sdk=$2 ;;
    --remove)
        ! in_sdk "$dest" \
            || die "AURORA_DLSS_DIR is inside NVIDIA's DLSS SDK ($dest): nothing removed, choose another"
        rm -f "$dest"/libnvidia-ngx-dlss.so.* "$dest"/libsl_ngx.so "$dest/$LICENCE_NAME"
        old_files
        rmdir "$dest" 2>/dev/null || true
        echo "Removed NVIDIA DLSS from $dest. The viewer offers FSR alone from its next start."
        exit 0 ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
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
        /LICENSE.txt /include/ /lib/Linux_x86_64/libnvsdk_ngx.a '/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.*'
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
# AURORA_DLSS_DIR inside an SDK (in_sdk), or the selected SDK's own folder
# whatever its shape
dest_real=$(realpath -m "$dest")
! in_sdk "$dest" || die "AURORA_DLSS_DIR is inside NVIDIA's DLSS SDK ($dest): choose another"
for d in "$sdk" "$sdk/lib/Linux_x86_64" "$sdk/lib/Linux_x86_64/rel"; do
    [[ $dest_real != "$(realpath -m "$d")" ]] \
        || die "AURORA_DLSS_DIR is the SDK's own folder $d: choose another"
done

echo
echo "NVIDIA's DLSS SDK is under NVIDIA's licence, not the viewer's. Read it:"
echo
${PAGER:-less} "$licence" || cat "$licence"
echo
read -r -p "Do you accept NVIDIA's licence for the DLSS SDK? Type yes to install: " answer || answer=
[[ $answer == yes ]] || die "not accepted: nothing installed"

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
staged=()
trap 'rm -rf "$work"; rm -f "${staged[@]}"' EXIT
stage() { # <source> <name> <mode>
    local tmp="$dest/.$2.aurora-new"
    staged+=("$tmp")
    install -m "$3" "$1" "$tmp" || die "cannot copy $2 to $dest: the install there is unchanged"
    cmp -s "$1" "$tmp" || die "the copy of $2 in $dest differs from its source: the install there is unchanged"
}
stage "$snippet" "$new_snippet" 755
stage "$src/libsl_ngx.so" libsl_ngx.so 755
stage "$licence" "$LICENCE_NAME" 644
for name in "$new_snippet" libsl_ngx.so "$LICENCE_NAME"; do
    mv -f "$dest/.$name.aurora-new" "$dest/$name"
done
staged=()
shopt -s nullglob
for old in "$dest"/libnvidia-ngx-dlss.so.*; do
    [[ $(basename "$old") == "$new_snippet" ]] || rm -f "$old"
done
shopt -u nullglob
old_files

echo
echo "Installed $(basename "$snippet") and libsl_ngx.so in $dest."
echo "Restart the viewer, then choose NVIDIA DLSS under Preferences > Graphics > Anti-aliasing."
echo "NVIDIA graphics cards only."
