#!/usr/bin/env bash
# Regenerate docs/reference/cli.md from the help texts of the cdept binary.
#
#   scripts/gen-cli-docs.sh [output-file]
#
# The program is $CDEPT_BIN (default: target/debug/cdept, else target/release/cdept,
# else `cdept` on PATH). Every command and subcommand is asked for `--help`; the output
# is one section per command. The options every command has (-p, -v, -h) are kept in
# the first section only.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:-$root/docs/reference/cli.md}

bin=${CDEPT_BIN:-}
if [ -z "$bin" ]; then
    for candidate in "$root/target/debug/cdept" "$root/target/release/cdept"; do
        if [ -x "$candidate" ]; then bin=$candidate; break; fi
    done
    bin=${bin:-cdept}
fi

# Names of the subcommands listed under "Commands:" of a help text (without `help`).
subcommands() {
    awk '
        /^Commands:/ { on = 1; next }
        on && /^[^ ]/ { on = 0 }
        on && /^  [a-z]/ { if ($1 != "help") print $1 }
    '
}

# Drop the options every command has, with their description lines.
strip_global() {
    awk '
        skip && (/^$/ || /^          /) { next }
        { skip = 0 }
        /^  (-p, --project|-v, --verbose|-h, --help)/ { skip = 1; next }
        { print }
    ' | awk '
        # an "Options:" heading with nothing under it is dropped
        pending != "" { if ($0 != "") { print pending; print } ; pending = ""; if ($0 != "") next; }
        /^Options:$/ { pending = $0; next }
        { print }
        END { if (pending != "") {} }
    ' | cat -s | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
}

section() { # level, words...
    local level=$1; shift
    local text
    text=$("$bin" "$@" --help 2>&1) || { echo "gen-cli-docs: '$bin $*' failed" >&2; exit 1; }
    local hashes
    hashes=$(printf '%*s' "$level" '' | tr ' ' '#')
    printf '%s %s\n\n```text\n' "$hashes" "cdept${*:+ $*}"
    if [ $# -eq 0 ]; then
        printf '%s\n' "$text"
    else
        printf '%s\n' "$text" | strip_global
    fi
    printf '```\n\n'
    local sub
    for sub in $(printf '%s\n' "$text" | subcommands); do
        section $((level + 1 > 6 ? 6 : level + 1)) "$@" "$sub"
    done
}

{
    cat <<'HEAD'
# Command reference

This page is generated from the program's own help texts by `scripts/gen-cli-docs.sh`;
do not edit it by hand. Every command has the options `-p, --project <PROJECT>`,
`-v, --verbose` and `-h, --help` (see the first section); they are left out of the
sections below. How the commands are organised is in [the command line guide](../guide/cli.md).

HEAD
    section 2
} > "$out.tmp"
mv "$out.tmp" "$out"
echo "wrote $out ($("$bin" version | head -1))"
