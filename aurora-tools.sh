#!/bin/sh
# Aurora Tools entry point on Linux, the counterpart of aurora-tools.cmd: keyboard
# menus, PR tracking and release management. Loads scripts/tools/aurora-tools.ps1
# from the repository or the project folder with PowerShell 7 and passes its
# arguments on, for example ./aurora-tools.sh -Action diagnose. Alone in a folder,
# downloads the installer and installs the project there. It follows symlinks.
self=$(readlink -f -- "$0") || exit 1
here=$(dirname -- "$self")
# Double-clicked: open a terminal, as Windows opens a console for a .cmd.
if [ $# -eq 0 ] && [ ! -t 0 ] && [ ! -t 1 ] && [ -z "$AURORA_TOOLS_IN_TERMINAL" ] && { [ -n "$WAYLAND_DISPLAY" ] || [ -n "$DISPLAY" ]; }; then
    export AURORA_TOOLS_IN_TERMINAL=1
    for t in ${TERMINAL:+"$TERMINAL -e"} xdg-terminal-exec "konsole -e" "gnome-terminal --" "kgx --" "ptyxis --" kitty foot "alacritty -e" "wezterm start --" "ghostty -e" "xterm -e"; do
        command -v "${t%% *}" >/dev/null 2>&1 && exec $t "$self"
    done
fi
if ! command -v pwsh >/dev/null 2>&1; then
    echo 'Aurora Tools: PowerShell 7 (pwsh) is needed.' >&2
    echo '  Arch: paru -S powershell-bin (AUR). Others: https://aka.ms/powershell' >&2
    exit 127
fi
cd "$here" || exit 1
# aurora-viewer/ in any case: Windows paths ignore it, and the folder can be cloned either way
for script in "$here/scripts/tools/aurora-tools.ps1" "$here/aurora-viewer/scripts/tools/aurora-tools.ps1" "$here/Aurora-Viewer/scripts/tools/aurora-tools.ps1"; do
    [ -f "$script" ] && exec pwsh -NoProfile -File "$script" "$@"
done
pwsh -NoProfile -Command '$f = Join-Path ([IO.Path]::GetTempPath()) "aurora-setup.ps1"; irm https://raw.githubusercontent.com/Aurora-Viewer/Aurora-Viewer/main/scripts/setup.ps1 -OutFile $f; & $f'
code=$?
printf 'Press Enter to close. '
read -r _
exit "$code"
