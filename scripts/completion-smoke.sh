#!/usr/bin/env bash
# Drive `rbx`'s dynamic completions in real shells and assert on the menu.
#
# The unit tests pin the contract between the two halves: each generated script
# contains its hook and asks for the right lister, and the listers print the
# right lines. What no test in the workspace can reach is the half that runs
# inside a shell, whether the graft survives that shell's own completion
# system. This is that half, and it is why #102 asked for it.
#
# Usage: scripts/completion-smoke.sh <path-to-rbx>
#
# Skips any shell that is not installed and says so, so it stays runnable on a
# developer machine that has two of the four. CI installs all four.

set -uo pipefail

RBX="${1:?usage: completion-smoke.sh <path-to-rbx>}"
RBX="$(cd "$(dirname "$RBX")" && pwd)/$(basename "$RBX")"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/rbxplace.toml" <<'TOML'
[owner]
type = "user"
id = 1234567890

[staging]
universe_id = 111111111
[staging.places]
main = 999999991
lobby = 999999992

[prod]
universe_id = 222222222
[prod.places]
main = 999999993
TOML

cd "$WORK"
# The hooks call `rbx`, so it has to be the binary under test and not whatever
# a developer happens to have installed.
mkdir -p "$WORK/bin"
# Copied rather than symlinked: on Windows a link named `rbx` is not something
# PowerShell will resolve through PATHEXT, and the point of the shim is that
# every shell finds the same binary by the name the hooks call.
cp "$RBX" "$WORK/bin/rbx"
case "$RBX" in
  *.exe) cp "$RBX" "$WORK/bin/rbx.exe" ;;
esac
export PATH="$WORK/bin:$PATH"

fail=0
ran=0

report() { # name, output, expected...
  local name="$1" out="$2"
  shift 2
  local missing=()
  for want in "$@"; do
    grep -qw -- "$want" <<<"$out" || missing+=("$want")
  done
  if [ ${#missing[@]} -eq 0 ]; then
    echo "  ok   $name"
  else
    echo "  FAIL $name: missing: ${missing[*]}"
    echo "       got: $(tr '\n' ' ' <<<"$out")"
    fail=1
  fi
}

echo "== bash"
if command -v bash >/dev/null; then
  ran=$((ran + 1))
  "$RBX" completions bash > "$WORK/rbx.bash"
  # Call the registered function the way bash does, then read COMPREPLY. This
  # is the whole completion path bar the keypress.
  out=$(bash --noprofile --norc -c '
    source "$1/rbx.bash"
    _drive() {
      COMP_WORDS=("rbx" "place" "upload" "$2" "")
      COMP_CWORD=4
      COMPREPLY=()
      "$(complete -p rbx | sed -E "s/.*-F ([^ ]+).*/\1/")" 2>/dev/null
      printf "%s\n" "${COMPREPLY[@]}"
    }
    _drive "" --env
    _drive "" --place
  ' _ "$WORK" 2>/dev/null)
  report "bash --env/--place" "$out" staging prod lobby
else
  echo "  skip (bash not installed)"
fi

echo "== fish"
if command -v fish >/dev/null; then
  ran=$((ran + 1))
  "$RBX" completions fish > "$WORK/rbx.fish"
  # fish answers "what would you complete here" directly, which is as close to
  # a keypress as a script can get.
  out=$(fish -c "
    source $WORK/rbx.fish
    complete -C 'rbx place upload --env '
    complete -C 'rbx place upload --place '
  " 2>/dev/null)
  report "fish --env/--place" "$out" staging prod lobby
else
  echo "  skip (fish not installed)"
fi

echo "== zsh"
if command -v zsh >/dev/null; then
  ran=$((ran + 1))
  "$RBX" completions zsh > "$WORK/_rbx"
  # zsh's completion system cannot be driven headlessly the way the other three
  # can. What is asserted instead is one step short of the menu: the helpers the
  # graft inserts produce the values, in a real zsh, from a real directory.
  out=$(zsh -f -c "
    fpath=($WORK \$fpath)
    source $WORK/_rbx 2>/dev/null
    _rbx_env_values 2>/dev/null || rbx env list --names
    _rbx_place_values 2>/dev/null || rbx env list --place-names
  " 2>/dev/null)
  report "zsh helpers" "$out" staging prod lobby
else
  echo "  skip (zsh not installed)"
fi

echo "== pwsh"
if command -v pwsh >/dev/null; then
  ran=$((ran + 1))
  "$RBX" completions powershell > "$WORK/rbx.ps1"
  # TabExpansion2 is what the shell itself calls on TAB.
  #
  # PATH is prepended inside pwsh rather than inherited: a path exported from
  # this script is POSIX-shaped on Windows and does not survive the crossing.
  # `cygpath` exists only there, which is exactly where the translation is
  # needed.
  bin_for_pwsh="$WORK/bin"
  if command -v cygpath >/dev/null; then bin_for_pwsh="$(cygpath -w "$WORK/bin")"; fi
  out=$(pwsh -NoProfile -Command "
    \$env:PATH = '$bin_for_pwsh' + [IO.Path]::PathSeparator + \$env:PATH
    . './rbx.ps1'
    foreach (\$line in @('rbx place upload --env ', 'rbx place upload --place ')) {
      (TabExpansion2 -inputScript \$line -cursorColumn \$line.Length).CompletionMatches |
        ForEach-Object { \$_.CompletionText }
    }
  " 2>/dev/null)
  report "pwsh --env/--place" "$out" staging prod lobby
else
  echo "  skip (pwsh not installed)"
fi

echo
if [ "$ran" -eq 0 ]; then
  echo "no shell was available; nothing was proven"
  exit 1
fi
if [ "$fail" -ne 0 ]; then
  echo "completion smoke: FAILED"
  exit 1
fi
echo "completion smoke: $ran shell(s) ok"
