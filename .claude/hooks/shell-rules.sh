#!/usr/bin/env bash
# PreToolUse hook of Claude Code (.claude/settings.json): runs before every
# command an agent sends through its Bash or PowerShell tool, and refuses the
# few that AGENTS.md forbids (section 5: "Tools", "Tests" and "Build slots"),
# naming the command to use instead. A rule in a file
# can be forgotten in a long session; this cannot. Only the agents' tool
# calls come here: never a human's own terminal.
#
# The hook's JSON arrives on stdin. Exit 3 refuses the call, with one line on
# stderr for Claude; exit 0 says nothing and leaves the call to the usual
# permission checks. The command line of .claude/settings.json turns that 3
# into the 2 Claude Code reads as a refusal, and any other code into 0 (a
# syntax error in an older bash is 2, a missing file 127): a broken hook
# lets everything through, it never refuses everything.
# Tested by scripts/test-agent-hook.ps1 (a step of check.ps1).
#
# It runs before every shell call, so:
# - builtins only (no cat, sed, jq: one more process each, slow on Windows);
# - whatever it does not understand goes through, and a missing tool (rg)
#   turns its rule off instead of refusing;
# - it reads commands where they start (after ; & | ( { and a new line), not
#   inside the text of another command: `git commit -m "… grep -r …"` passes.
# Bash 3.2 (macOS) to 5 (Git Bash): no ${var,,}, no associative array.

LC_ALL=C
shopt -s nocasematch

IFS= read -r -d '' input
case $input in
    *grep* | *findstr* | *select-string* | *sls* | *cargo* | *rustc_wrapper* | *rustc-wrapper*) ;;
    *) exit 0 ;;
esac

nl=$'\n'
dq='"'
sq="'"
backslash='\'
# stand-ins for an escaped backslash and an escaped quote of the JSON text
esc_backslash=$'\001'
esc_quote=$'\002'

# tool_input.command, as plain text. Once the escaped quotes are set aside,
# the quotes left are the JSON's own: the value ends at the next one.
text=${input//\\\\/$esc_backslash}
text=${text//\\\"/$esc_quote}
case $text in
    *\"command\"*) ;;
    *) exit 0 ;;
esac
text=${text#*\"command\"}
text=${text#*\"}
cmd=${text%%\"*}
cmd=${cmd//\\n/$nl}
cmd=${cmd//\\r/}
cmd=${cmd//\\t/ }
cmd=${cmd//$esc_quote/$dq}
cmd=${cmd//$esc_backslash/$backslash}

refuse() {
    # Claude Code shows it after "hook error: [this command]:"
    printf '%s\n' "Refused by a rule of this repository, not a failure: $1" >&2
    exit 3
}

# The search rules only hold where ripgrep is installed (aurora-tools,
# "Repair / update"); without it the old commands are all there is.
have_rg=
rg_installed() {
    if [[ -z $have_rg ]]; then
        if command -v rg > /dev/null 2>&1; then have_rg=yes; else have_rg=no; fi
    fi
    [[ $have_rg == yes ]]
}
use_rg='recursive text search goes through ripgrep, for example: rg -n "pattern" <folder> (AGENTS.md section 5, "Tools").'

# --- The compiler wrapper (AGENTS.md sections 3 and 5, "Build slots") -----------
# <project folder>\.cargo\config.toml sets it for every build of the PC: the
# build gate, then the shared compiler cache. The environment variable and
# --config both win over that file, and so go around the gate.
re_wrapper_env='(^|[^a-z0-9_"'$sq'])(cargo_build_)?rustc_wrapper='
re_wrapper_ps='\$env:(cargo_build_)?rustc_wrapper[[:space:]]*='
re_wrapper_config='--config[[:space:]=]+["'$sq']?build\.rustc-wrapper'
if [[ $cmd =~ $re_wrapper_env || $cmd =~ $re_wrapper_ps || $cmd =~ $re_wrapper_config ]]; then
    refuse 'leave the rustc wrapper of Cargo to <project folder>\.cargo\config.toml, which carries the build gate and the shared compiler cache: no RUSTC_WRAPPER, no --config build.rustc-wrapper (AGENTS.md section 5, "Build slots").'
fi

# --- Get-ChildItem -Recurse into Select-String (PowerShell) ---------------------
case $cmd in
    *select-string* | *sls*)
        list='(^|[^a-z0-9_.-])(get-childitem|gci|ls|dir)[[:space:]]+([^;|'$nl']*[[:space:]])?-(recurse|recurs|recur|recu|rec|re|r|s)'
        re_piped=$list'([[:space:]:][^;'$nl']*)?\|[[:space:]]*(select-string|sls)([[:space:]]|$)'
        re_argument='(select-string|sls)[[:space:]][^;|'$nl']*'$list'([^a-z]|$)'
        if [[ $cmd =~ $re_piped || $cmd =~ $re_argument ]] && rg_installed; then
            refuse "$use_rg"
        fi
        ;;
esac

# --- Command by command ---------------------------------------------------------
# What may stand before the program: a wrapper word, VAR=value assignments,
# `$result =`. Then its folder (group 5), its name (6) and its arguments (7).
re_command='^[[:space:]]*((time|sudo|command|exec|env)[[:space:]]+)*([a-z_][a-z0-9_]*=[^[:space:]]*[[:space:]]+)*(\$[a-z_][a-z0-9_:]*[[:space:]]*=[[:space:]]*)?([^[:space:]]*[/\\])?([^[:space:]/\\]+)(.*)$'
re_grep_recursive='(^|[[:space:]])-(-recursive|-dereference-recursive|[a-z]*r[a-z]*)([[:space:]=]|$)'
re_findstr_recursive='(^|[[:space:]])/[a-z]*s[a-z]*([[:space:]:]|$)'
# cargo-fuzz only builds on nightly, in fuzz/, its own workspace and target
# (docs/TESTING.md, "Fuzzing"): the one toolchain override let through.
re_fuzz='^\+[^[:space:]]+[[:space:]]+fuzz([[:space:]]|$)'
re_test='^(-[^[:space:]]+[[:space:]]+)*(test|t)([[:space:]]|$)'
re_package='[[:space:]](-p|--package)([[:space:]=]|[a-z0-9_])'
re_whole='[[:space:]]--(workspace|all|manifest-path)[[:space:]=]'

separators='[;&|({`]'
while IFS= read -r part; do
    case $part in
        *grep* | *findstr* | *cargo*) ;;
        *) continue ;;
    esac
    [[ $part =~ $re_command ]] || continue
    name=${BASH_REMATCH[6]}
    args=${BASH_REMATCH[7]}
    case $name in *.exe) name=${name%.???} ;; esac
    case $name in
        grep | egrep | fgrep)
            if [[ $args =~ $re_grep_recursive ]] && rg_installed; then refuse "$use_rg"; fi
            ;;
        rgrep)
            if rg_installed; then refuse "$use_rg"; fi
            ;;
        findstr)
            if [[ $args =~ $re_findstr_recursive ]] && rg_installed; then refuse "$use_rg"; fi
            ;;
        cargo)
            args=${args#"${args%%[![:space:]]*}"}
            case $args in
                +*)
                    [[ $args =~ $re_fuzz ]] && continue
                    refuse 'build with the toolchain of rust-toolchain.toml, without "cargo +<toolchain>": another one recompiles the whole target folder and takes nothing from the shared cache.'
                    ;;
            esac
            if [[ $args =~ $re_test ]]; then
                # the arguments of Cargo itself: what follows " -- " goes to the tests
                own=" ${args%% -- *} "
                if [[ $own =~ $re_package && ! $own =~ $re_whole ]]; then
                    refuse 'run "cargo test --workspace <filter>" instead of "cargo test -p <crate>": one crate alone resolves the features of its dependencies differently, and Cargo builds them a second time (AGENTS.md section 5).'
                fi
            fi
            ;;
    esac
done <<< "${cmd//$separators/$nl}"
exit 0
