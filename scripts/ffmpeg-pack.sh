#!/usr/bin/env bash
# The ffmpeg pack of the Director (aurora-updater ffmpeg_pack.rs): ffmpeg and
# the three libraries it encodes with, built from the pinned sources below by
# the "FFmpeg pack" workflow (.github/workflows/ffmpeg-pack.yml). This file is
# also the build script of the pack's corresponding source: the source
# archive holds it beside the four trees.
#
#   ffmpeg-pack.sh fetch   <sources>
#   ffmpeg-pack.sh build   <target> <sources> <work>      -> <work>/prefix/bin/ffmpeg
#   ffmpeg-pack.sh package <system> <ffmpeg> <sources> <out> <version>
#   ffmpeg-pack.sh verify  <ffmpeg> <work>
#   ffmpeg-pack.sh source  <sources> <out> <version>
#
# target: linux-x64 (static, native), windows-x64 (static, cross-built with
# MinGW-w64 on Linux), macos-arm64 / macos-x64 (native on each, joined by
# lipo into macos-universal by the workflow). system: linux-x64,
# windows-x64, macos-universal (the names ffmpeg_pack.rs downloads).
#
# The build: ffmpeg under the GPL version 3 (--enable-gpl --enable-version3,
# for x264), nothing found by itself (--disable-autodetect): x264 (H.264),
# libvpx (VP9) and libopus (Opus), plus ffmpeg's own encoders the Director
# uses (ProRes, FFV1, AAC, FLAC, PCM), zlib, the system's threads and its
# hardware H.264 encoder (Media Foundation, VideoToolbox). No network, no
# ffplay nor ffprobe: the viewer only runs ffmpeg on local files and pipes.
# No nonfree code: the pack can be redistributed.
set -euo pipefail

# Pinned sources: a commit each, checked once fetched. A new version changes
# these lines and the workflow's PACK_VERSION.
FFMPEG_VERSION=9.0.2
FFMPEG_URL=https://github.com/FFmpeg/FFmpeg.git
FFMPEG_COMMIT=946fcce07b6dcd0331c8cc609192aeff5e1924f8 # tag n9.0.2
X264_URL=https://code.videolan.org/videolan/x264.git
X264_COMMIT=b35605ace3ddf7c1a5d67a2eb553f034aef41d55 # branch stable
VPX_VERSION=1.17.0
VPX_URL=https://github.com/webmproject/libvpx.git
VPX_COMMIT=6df3ec34557879fff673706f4a1d9fbd0f3a6f0e # tag v1.17.0
OPUS_VERSION=1.6.1
OPUS_URL=https://github.com/xiph/opus.git
OPUS_COMMIT=22244de5a79bd1d6d623c32e72bf1954b56235be # tag v1.6.1

# The oldest macOS the viewer opens on (packaging/macos/Info.plist).
MACOS_MIN=13.0

die() { echo "ffmpeg-pack: $*" >&2; exit 1; }
# empty arrays under set -u need bash 4.4 (macOS has 3.2 without Homebrew's)
[ "${BASH_VERSINFO[0]}" -ge 5 ] || { [ "${BASH_VERSINFO[0]}" -eq 4 ] && [ "${BASH_VERSINFO[1]}" -ge 4 ]; } ||
    die "bash 4.4 or later needed"
ncpu() { getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4; }

# fetch <url> <commit> <dir>: that commit alone, checked.
fetch_one() {
    local url=$1 commit=$2 dir=$3
    rm -rf "$dir"
    git init -q "$dir"
    git -C "$dir" fetch -q --depth 1 "$url" "$commit"
    git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
    [ "$(git -C "$dir" rev-parse HEAD)" = "$commit" ] || die "$url: not commit $commit"
}

cmd_fetch() {
    local src=${1:?sources}
    mkdir -p "$src"
    fetch_one "$FFMPEG_URL" "$FFMPEG_COMMIT" "$src/ffmpeg"
    fetch_one "$X264_URL" "$X264_COMMIT" "$src/x264"
    fetch_one "$VPX_URL" "$VPX_COMMIT" "$src/libvpx"
    fetch_one "$OPUS_URL" "$OPUS_COMMIT" "$src/opus"
    # a shallow clone has no tag to name the version with: their own files say it
    echo "$FFMPEG_VERSION" > "$src/ffmpeg/VERSION"
    echo "PACKAGE_VERSION=\"$OPUS_VERSION\"" > "$src/opus/package_version"
}

cmd_build() {
    local target=${1:?target} src work
    src=$(cd "${2:?sources}" && pwd)
    mkdir -p "${3:?work}"
    work=$(cd "$3" && pwd)
    local prefix=$work/prefix j
    j=$(ncpu)
    rm -rf "$work/build" "$prefix"
    mkdir -p "$work/build" "$prefix"

    local x264_args=() vpx_args=() opus_args=() ff_args=() vpx_env=()
    case $target in
        linux-x64)
            vpx_args=(--target=x86_64-linux-gcc --as=nasm --enable-pic)
            ff_args=(--enable-pthreads --extra-ldexeflags=-static)
            ;;
        windows-x64)
            local host=x86_64-w64-mingw32
            x264_args=(--host=$host --cross-prefix=$host-)
            vpx_env=(CROSS=$host-)
            vpx_args=(--target=x86_64-win64-gcc --as=nasm)
            # MinGW has no libssp: no stack protector nor fortify in opus
            opus_args=(-DCMAKE_SYSTEM_NAME=Windows -DCMAKE_C_COMPILER=$host-gcc -DCMAKE_RC_COMPILER=$host-windres
                -DCMAKE_FIND_ROOT_PATH=/usr/$host -DOPUS_STACK_PROTECTOR=OFF -DOPUS_FORTIFY_SOURCE=OFF)
            # static: no libwinpthread or libgcc DLL beside ffmpeg.exe
            ff_args=(--target-os=mingw32 --arch=x86_64 --cross-prefix=$host- --enable-cross-compile
                --enable-w32threads --enable-mediafoundation --extra-ldexeflags=-static)
            ;;
        macos-arm64 | macos-x64)
            export MACOSX_DEPLOYMENT_TARGET=$MACOS_MIN
            local arch=arm64
            [ "$target" = macos-x64 ] && arch=x86_64
            [ "$(uname -m)" = "$arch" ] || die "$target is built on a $arch Mac"
            [ "$arch" = x86_64 ] && vpx_args=(--as=nasm)
            vpx_args+=(--enable-pic)
            opus_args=(-DCMAKE_OSX_DEPLOYMENT_TARGET=$MACOS_MIN)
            ff_args=(--enable-pthreads --enable-videotoolbox --enable-audiotoolbox
                --extra-cflags=-mmacosx-version-min=$MACOS_MIN --extra-ldflags=-mmacosx-version-min=$MACOS_MIN)
            ;;
        *) die "unknown target $target" ;;
    esac

    echo "::group::x264"
    cp -R "$src/x264" "$work/build/x264"
    (cd "$work/build/x264" &&
        ./configure --prefix="$prefix" --enable-static --enable-pic --disable-cli --disable-opencl "${x264_args[@]}" &&
        make -j"$j" && make install)
    echo "::endgroup::"

    echo "::group::libvpx"
    mkdir -p "$work/build/libvpx"
    (cd "$work/build/libvpx" &&
        env "${vpx_env[@]}" "$src/libvpx/configure" --prefix="$prefix" --enable-static --disable-shared \
            --enable-vp9-highbitdepth --disable-examples --disable-tools --disable-docs --disable-unit-tests \
            "${vpx_args[@]}" &&
        make -j"$j" && make install)
    echo "::endgroup::"

    echo "::group::opus"
    cmake -S "$src/opus" -B "$work/build/opus" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$prefix" \
        -DBUILD_SHARED_LIBS=OFF -DOPUS_BUILD_SHARED_LIBRARY=OFF -DOPUS_BUILD_TESTING=OFF -DOPUS_BUILD_PROGRAMS=OFF \
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON "${opus_args[@]}"
    cmake --build "$work/build/opus" -j "$j"
    cmake --install "$work/build/opus"
    echo "::endgroup::"

    echo "::group::ffmpeg"
    mkdir -p "$work/build/ffmpeg"
    # only these libraries' .pc files, never the build machine's
    (cd "$work/build/ffmpeg" &&
        PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" PKG_CONFIG_PATH= "$src/ffmpeg/configure" --prefix="$prefix" \
            --enable-gpl --enable-version3 --disable-autodetect \
            --enable-static --disable-shared --disable-debug --disable-doc --disable-ffplay --disable-ffprobe \
            --disable-network --enable-libx264 --enable-libvpx --enable-libopus --enable-zlib \
            --pkg-config=pkg-config --pkg-config-flags=--static \
            --extra-cflags="-I$prefix/include" --extra-ldflags="-L$prefix/lib" "${ff_args[@]}" &&
        make -j"$j" && make install)
    echo "::endgroup::"
    local exe=$prefix/bin/ffmpeg
    [ "$target" = windows-x64 ] && exe=$exe.exe
    [ -f "$exe" ] || die "no $exe"
    echo "built $exe"
}

# package <system> <ffmpeg> <sources> <out> <version>: the folder the viewer
# unpacks (ffmpeg_pack.rs: aurora-ffmpeg.json, bin/, the licences), zipped
# with the Unix permissions kept, and its SHA-256.
cmd_package() {
    local system=${1:?system} exe=${2:?ffmpeg} src=${3:?sources} out=${4:?out} version=${5:?version}
    local name=Aurora-FFmpeg-$version-$system
    local dir=$out/$name program=ffmpeg
    [ "$system" = windows-x64 ] && program=ffmpeg.exe
    rm -rf "$dir" "$out/$name.zip"
    mkdir -p "$dir/bin" "$dir/LICENSES"
    cp "$exe" "$dir/bin/$program"
    chmod 755 "$dir/bin/$program"
    printf '{"schema": 1, "version": "%s"}\n' "$version" > "$dir/aurora-ffmpeg.json"
    cp "$src/ffmpeg/COPYING.GPLv3" "$dir/LICENSES/ffmpeg-COPYING.GPLv3.txt"
    cp "$src/ffmpeg/LICENSE.md" "$dir/LICENSES/ffmpeg-LICENSE.md"
    cp "$src/x264/COPYING" "$dir/LICENSES/x264-COPYING.txt"
    cp "$src/libvpx/LICENSE" "$dir/LICENSES/libvpx-LICENSE.txt"
    cp "$src/libvpx/PATENTS" "$dir/LICENSES/libvpx-PATENTS.txt"
    cp "$src/opus/COPYING" "$dir/LICENSES/opus-COPYING.txt"
    cat > "$dir/README.txt" <<EOF
Aurora Viewer's ffmpeg pack $version ($system)

The ffmpeg program the Director of Aurora Viewer records its films with. The
viewer downloads this archive when the user asks for it, checks its size and
SHA-256, and runs bin/$program only to encode the Director's films.

ffmpeg $FFMPEG_VERSION (FFmpeg/FFmpeg@$FFMPEG_COMMIT), built with
x264 (videolan/x264@$X264_COMMIT), libvpx $VPX_VERSION
(webmproject/libvpx@$VPX_COMMIT) and libopus $OPUS_VERSION
(xiph/opus@$OPUS_COMMIT), linked statically.

License: this build of ffmpeg is under the GNU General Public License,
version 3 (LICENSES/ffmpeg-COPYING.GPLv3.txt), as x264 is under the GPL
version 2 or later. libvpx and libopus are under BSD licenses
(LICENSES/libvpx-LICENSE.txt and libvpx-PATENTS.txt, LICENSES/opus-COPYING.txt).

Corresponding source: Aurora-FFmpeg-$version-source.tar.gz, attached to the
same release (https://github.com/Aurora-Viewer/Aurora-Viewer/releases/tag/ffmpeg-$version):
the four source trees and the build script (ffmpeg-pack.sh). The configure
line of this build is printed by: ffmpeg -buildconf
EOF
    (cd "$out" && zip -r -q -X "$name.zip" "$name")
    local hash
    hash=$( (sha256sum "$out/$name.zip" 2>/dev/null || shasum -a 256 "$out/$name.zip") | cut -d' ' -f1)
    echo "$hash  $name.zip" > "$out/$name.zip.sha256"
    echo "$name.zip: $(wc -c < "$out/$name.zip" | tr -d ' ') bytes, sha256 $hash"
}

# verify <ffmpeg> <work>: every encoder the Director asks for (recorder.rs)
# is there, and each format makes a film that reads back.
cmd_verify() {
    local exe=${1:?ffmpeg} work=${2:?work}
    mkdir -p "$work"
    "$exe" -hide_banner -version | head -1
    local list
    list=$("$exe" -hide_banner -encoders)
    local encoder
    for encoder in libx264 prores_ks libvpx-vp9 ffv1 aac pcm_s16le libopus flac; do
        grep -qE "^ [A-Z.]{6} $encoder( |$)" <<< "$list" || die "no $encoder encoder"
    done
    case "$("$exe" -hide_banner -buildconf)" in
        *--enable-nonfree*) die "a nonfree build cannot be published" ;;
    esac
    local video=(-f lavfi -i testsrc2=size=320x240:rate=30 -f lavfi -i sine=frequency=440:sample_rate=48000 -t 1)
    "$exe" -hide_banner -loglevel error -y "${video[@]}" -c:v libx264 -pix_fmt yuv420p -c:a aac "$work/test.mp4"
    "$exe" -hide_banner -loglevel error -y "${video[@]}" -c:v libvpx-vp9 -b:v 1M -c:a libopus "$work/test.webm"
    "$exe" -hide_banner -loglevel error -y "${video[@]}" -c:v prores_ks -profile:v 4444 -pix_fmt yuva444p10le -c:a pcm_s16le "$work/test.mov"
    "$exe" -hide_banner -loglevel error -y "${video[@]}" -c:v ffv1 -level 3 -c:a flac "$work/test.mkv"
    local film
    for film in test.mp4 test.webm test.mov test.mkv; do
        "$exe" -hide_banner -loglevel error -xerror -i "$work/$film" -f null - || die "$film does not read back"
        echo "$film: $(wc -c < "$work/$film" | tr -d ' ') bytes, reads back"
    done
}

# source <sources> <out> <version>: the corresponding source of the pack.
cmd_source() {
    local src=${1:?sources} out=${2:?out} version=${3:?version}
    local name=Aurora-FFmpeg-$version-source
    local stage
    stage=$(mktemp -d)
    mkdir -p "$stage/$name"
    local tree
    for tree in ffmpeg x264 libvpx opus; do
        git -C "$src/$tree" archive --format=tar --prefix="$tree/" HEAD | tar -x -C "$stage/$name"
    done
    cp "$src/ffmpeg/VERSION" "$stage/$name/ffmpeg/VERSION"
    cp "$src/opus/package_version" "$stage/$name/opus/package_version"
    cp "${BASH_SOURCE[0]}" "$stage/$name/ffmpeg-pack.sh"
    cat > "$stage/$name/README.txt" <<EOF
Corresponding source of Aurora Viewer's ffmpeg pack $version.

ffmpeg/  FFmpeg/FFmpeg@$FFMPEG_COMMIT ($FFMPEG_VERSION)
x264/    videolan/x264@$X264_COMMIT
libvpx/  webmproject/libvpx@$VPX_COMMIT ($VPX_VERSION)
opus/    xiph/opus@$OPUS_COMMIT ($OPUS_VERSION)

Unmodified, apart from ffmpeg/VERSION and opus/package_version, which name
their version where a Git checkout would. To build: ffmpeg-pack.sh build
<target> . <work folder> (see the head of the script for the targets and
the tools each needs).
EOF
    mkdir -p "$out"
    tar -czf "$out/$name.tar.gz" -C "$stage" "$name"
    rm -rf "$stage"
    echo "$name.tar.gz: $(wc -c < "$out/$name.tar.gz" | tr -d ' ') bytes"
}

command=${1:-}
shift || true
case $command in
    fetch) cmd_fetch "$@" ;;
    build) cmd_build "$@" ;;
    package) cmd_package "$@" ;;
    verify) cmd_verify "$@" ;;
    source) cmd_source "$@" ;;
    *) die "usage: ffmpeg-pack.sh fetch|build|package|verify|source ..." ;;
esac
