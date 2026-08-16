//! `rbx completions <shell>`: the static script clap can write, plus a hook
//! that asks the binary for the values clap cannot know.
//!
//! Two thirds of a useful completion for this CLI lives in `rbxplace.toml`.
//! `--env` takes a section name from that file and `--place` takes a key from
//! that section's `places` table, so a script generated at build time can only
//! offer file names for both — which is worse than nothing, because it looks
//! like it worked.
//!
//! ## Why not clap's own dynamic completions
//!
//! `clap_complete` does ship a dynamic engine, where the shell calls the
//! binary back with the words typed so far and the binary answers. As of
//! 4.6.5 it is behind the `unstable-dynamic` feature and documented as
//! semver-exempt: a patch release may change or remove it. This binary is
//! distributed prebuilt with a declared MSRV, so a dependency that can break
//! on a patch bump costs a release, and the thing bought is a convenience.
//!
//! So the callback stays, and the coupling does not. The generated script
//! shells back into `rbx env list --names` and `rbx env list --place-names`,
//! two documented commands whose output is one value per line. If clap
//! stabilises its engine later, these hooks are the only thing to delete.
//!
//! ## What the hook must never do
//!
//! Completion runs wherever the user is standing, which is very often not a
//! project directory. `rbxplace.toml` may be absent, unreadable, or malformed,
//! and every one of those cases must produce *nothing*: no error text, no
//! warning, no non-zero status the shell reports. A completion that prints is
//! worse than a completion that is empty, because it lands in the middle of a
//! half-typed command line. Every hook therefore sends stderr to the void and
//! ignores the exit status — `rbx env list` already prints its diagnostics
//! there and its values on stdout, so discarding one stream is enough.
//!
//! The hook is also the reason `--env`/`--place` values are never synthesised
//! shell-side. `all` is a valid `--env` and is not in the file, and it would
//! be one line to add to each of the four hooks — four copies of a rule that
//! then has to stay true. The hooks pipe one command's stdout and decide
//! nothing, so they cannot drift from the CLI.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use clap::Command;
use clap_complete::Shell;

/// Build the completion script for `shell`.
///
/// With `dynamic` false the output is exactly what `clap_complete` emits, byte
/// for byte — the escape hatch for anyone who does not want a subprocess on
/// every `<TAB>`, and the baseline the tests compare against.
pub fn script(shell: Shell, cmd: &mut Command, dynamic: bool) -> String {
    let mut buf = Vec::new();
    clap_complete::generate(shell, cmd, "rbx", &mut buf);
    let base = String::from_utf8(buf).expect("clap_complete emits UTF-8");
    if dynamic {
        with_hook(shell, base)
    } else {
        base
    }
}

/// Write `script` to `path`, creating the parent directory if needed.
pub fn write(path: &Path, script: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating directory {}", parent.display()))?;
        }
    }
    let mut file =
        std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?;
    file.write_all(script.as_bytes())
        .with_context(|| format!("writing {}", path.display()))
}

/// Graft the dynamic-value hook onto a generated script.
///
/// Each shell gets the form its completion system actually supports, rather
/// than one shape forced onto four systems:
///
/// - bash and fish take an *addition*. Both let a later registration win or
///   merge, so the generated function is untouched and the hook sits at the
///   end of the file where it can be read and deleted.
/// - zsh takes a *substitution*. Its per-option actions are strings inside a
///   single `_arguments` call, so the only way in is to replace the action.
///   That is also the best outcome available: zsh then completes `--env` with
///   a real action, keeping its own grouping and description machinery.
/// - powershell takes an *insertion*. `Register-ArgumentCompleter` replaces
///   any previous registration for the same command, so a wrapper would delete
///   clap's completions rather than extend them; the hook goes inside the
///   generated block instead and returns before the static switch.
fn with_hook(shell: Shell, base: String) -> String {
    match shell {
        Shell::Bash => format!("{base}{}", lf(BASH_HOOK)),
        Shell::Fish => format!("{base}{}", lf(FISH_HOOK)),
        Shell::Zsh => zsh_hook(base),
        Shell::PowerShell => powershell_hook(base),
        // `Shell` is `#[non_exhaustive]`. A shell added by a future
        // `clap_complete` still gets its static script, which is what it gets
        // today; it just does not get the hook until somebody writes one.
        _ => base,
    }
}

/// A hook with its line endings pinned to LF.
///
/// `.gitattributes` normalises the repository to LF but checks the working
/// tree out in the platform's ending, so on a Windows clone these constants
/// arrive with CRLF baked into the literal — and a bash script with CRLF fails
/// on its first line with `$'\r': command not found`. Where the binary was
/// built must not decide whether the script it writes runs, so the endings are
/// fixed here rather than left to the checkout. clap's own output is already
/// LF on every platform.
fn lf(hook: &str) -> String {
    hook.replace("\r\n", "\n")
}

/// zsh: point the `--env` and `--place` actions at our own helpers, and define
/// those helpers near the top of the file.
///
/// The action strings clap emits are `:ENV:_default` and `:PLACE:_default`,
/// repeated once per subcommand context because both flags are global. Every
/// occurrence is rewritten, so the completion works after any subcommand and
/// not only at the top level. `--places` (the path to the file) renders as
/// `:PLACES:_files` and is deliberately not matched: it wants file names.
///
/// The helpers are inserted after the `#compdef rbx` line rather than appended,
/// because the generated file ends by *calling* `_rbx` when it is autoloaded.
/// Anything after that call is defined one invocation too late.
fn zsh_hook(base: String) -> String {
    let routed = base
        .replace(":ENV:_default", ":ENV:_rbx_env_names")
        .replace(":PLACE:_default", ":PLACE:_rbx_place_names");
    match routed.find('\n') {
        Some(eol) => {
            let (compdef, rest) = routed.split_at(eol + 1);
            format!("{compdef}{}{rest}", lf(ZSH_HOOK))
        }
        None => format!("{routed}{}", lf(ZSH_HOOK)),
    }
}

/// powershell: put the hook immediately after the script block's `param(...)`,
/// so it runs before the generated `switch` builds the static candidate list.
///
/// Falls back to leaving the script alone if the marker is gone — a completion
/// missing its dynamic values still completes every command and flag, whereas
/// text spliced at a guessed offset produces a script that does not parse, and
/// a broken profile is a much worse failure than a missing feature.
fn powershell_hook(base: String) -> String {
    const MARKER: &str = "param($wordToComplete, $commandAst, $cursorPosition)\n";
    match base.find(MARKER) {
        Some(at) => {
            let (head, tail) = base.split_at(at + MARKER.len());
            format!("{head}{}{tail}", lf(POWERSHELL_HOOK))
        }
        None => base,
    }
}

/// bash: wrap the generated `_rbx` instead of editing it.
///
/// `_rbx` is regenerated from the clap definition on every release, so nothing
/// may depend on its internals. The wrapper calls it, keeps whatever it
/// produced, and overrides `COMPREPLY` only for the two options we know
/// better. Re-registering afterwards is what makes the wrapper win: bash keeps
/// the last `complete` for a command name.
///
/// `IFS` is pinned to a newline for the substitution, so a place name
/// containing a space survives as one candidate. `compopt` turns off the
/// file-name fallback the registration asks for, so outside a project the
/// answer is nothing rather than a directory listing offered where an env name
/// belongs. It is redirected because bash 3.2 — still what macOS ships — has
/// no `compopt`; there the fallback stays, which is the behaviour that shell
/// had before this hook.
const BASH_HOOK: &str = r#"
# --- rbx dynamic values -----------------------------------------------------
# `--env` and `--place` take names from ./rbxplace.toml, which a script
# generated at build time cannot know. Ask the binary instead. Errors are
# discarded on purpose: completion runs in whatever directory you are standing
# in, and a missing or malformed rbxplace.toml must complete to nothing rather
# than print into a half-typed command line.
_rbx_dynamic() {
    _rbx "$@"

    local cur prev
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    local IFS=$'\n'
    case "${prev}" in
        -e|--env)
            compopt +o default +o bashdefault 2>/dev/null
            COMPREPLY=($(compgen -W "$(rbx env list --names 2>/dev/null)" -- "${cur}"))
            ;;
        --place)
            compopt +o default +o bashdefault 2>/dev/null
            COMPREPLY=($(compgen -W "$(rbx env list --place-names 2>/dev/null)" -- "${cur}"))
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _rbx_dynamic -o nosort -o bashdefault -o default rbx
else
    complete -F _rbx_dynamic -o bashdefault -o default rbx
fi
"#;

/// zsh helpers. `_describe` is what gives the values zsh's own grouping and
/// menu behaviour, so they list like any other completion rather than like raw
/// words. Returning 1 on an empty list hands the decision back to zsh, which
/// then shows nothing at all — the intended answer outside a project.
const ZSH_HOOK: &str = r#"
# --- rbx dynamic values -----------------------------------------------------
# `--env` and `--place` take names from ./rbxplace.toml. Errors are discarded:
# completion runs wherever you are standing, and a missing or malformed file
# must complete to nothing rather than print into a half-typed command line.
_rbx_env_names() {
    local -a names
    names=(${(f)"$(rbx env list --names 2>/dev/null)"})
    names=(${names:#})
    (( ${#names} )) || return 1
    _describe -t rbx-envs 'env' names
}

_rbx_place_names() {
    local -a names
    names=(${(f)"$(rbx env list --place-names 2>/dev/null)"})
    names=(${names:#})
    (( ${#names} )) || return 1
    _describe -t rbx-places 'place' names
}
"#;

/// fish: additive registrations. fish merges every `complete` that applies, so
/// naming the option again adds candidates to what clap already declared
/// instead of replacing it. `-f` is what stops fish from padding the list with
/// file names, and no `-n` condition is given because both options are global.
const FISH_HOOK: &str = r#"
# --- rbx dynamic values -----------------------------------------------------
# `--env` and `--place` take names from ./rbxplace.toml. Errors are discarded:
# completion runs wherever you are standing, and a missing or malformed file
# must complete to nothing rather than print into a half-typed command line.
complete -c rbx -s e -l env -r -f -a "(rbx env list --names 2>/dev/null)"
complete -c rbx -l place -r -f -a "(rbx env list --place-names 2>/dev/null)"
"#;

/// powershell: the previous token decides, so the hook walks the parsed
/// command line for the last element that is not the word being completed.
/// That handles both `rbx --env <TAB>` (nothing typed yet) and `rbx --env pr<TAB>`
/// (a prefix that is itself an element) without special-casing either.
///
/// Failure is swallowed twice over, because there are two ways to fail:
/// `SilentlyContinue` covers what the child process writes to stderr, and the
/// `try` covers `rbx` not being on `PATH` at all, which throws.
///
/// The early return happens whether or not any value was found. Falling
/// through to the generated switch would be the smaller diff, but the switch
/// answers a value position with the list of flags and subcommands — thirty
/// candidates, none of them valid there. Once the previous token is known to
/// be `--env` or `--place`, an empty list is the honest answer.
const POWERSHELL_HOOK: &str = r#"
    # --- rbx dynamic values -------------------------------------------------
    # `--env` and `--place` take names from ./rbxplace.toml, which a script
    # generated at build time cannot know. Ask the binary instead, and return
    # before the static list below. Errors are discarded on purpose: completion
    # runs in whatever directory you are standing in, and a missing or
    # malformed rbxplace.toml must complete to nothing rather than print into a
    # half-typed command line.
    $rbxPreviousToken = ''
    foreach ($rbxElement in $commandAst.CommandElements) {
        $rbxText = $rbxElement.ToString()
        if ($rbxText -ne $wordToComplete) {
            $rbxPreviousToken = $rbxText
        }
    }
    if ($rbxPreviousToken -eq '--env' -or $rbxPreviousToken -eq '-e' -or $rbxPreviousToken -eq '--place') {
        $ErrorActionPreference = 'SilentlyContinue'
        $rbxValues = @()
        try {
            if ($rbxPreviousToken -eq '--place') {
                $rbxValues = @(rbx env list --place-names 2>$null)
            } else {
                $rbxValues = @(rbx env list --names 2>$null)
            }
        } catch {
            $rbxValues = @()
        }
        return @($rbxValues |
            Where-Object { $_ -like "$wordToComplete*" } |
            ForEach-Object {
                [CompletionResult]::new($_, $_, [CompletionResultType]::ParameterValue, $_)
            })
    }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// The generated script, for real, from the real CLI definition. Testing
    /// against a fixture would prove only that the fixture still matches
    /// itself; the failure mode worth catching is `clap_complete` changing the
    /// text these hooks attach to.
    fn generated(shell: Shell) -> String {
        script(shell, &mut crate::Cli::command(), true)
    }

    fn static_only(shell: Shell) -> String {
        script(shell, &mut crate::Cli::command(), false)
    }

    /// The lines the hooks expect on stdout come from these two invocations
    /// and nothing else. Pinned per shell so a rename of either flag fails
    /// here rather than in somebody's prompt.
    const ENV_LISTER: &str = "env list --names";
    const PLACE_LISTER: &str = "env list --place-names";

    #[test]
    fn every_hooked_shell_calls_both_listers() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
            let out = generated(shell);
            assert!(
                out.contains(ENV_LISTER),
                "{shell} hook should call `rbx {ENV_LISTER}`"
            );
            assert!(
                out.contains(PLACE_LISTER),
                "{shell} hook should call `rbx {PLACE_LISTER}`"
            );
        }
    }

    #[test]
    fn no_dynamic_leaves_the_clap_script_untouched() {
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
            let out = static_only(shell);
            assert!(
                !out.contains(ENV_LISTER) && !out.contains(PLACE_LISTER),
                "{shell} --no-dynamic should emit no hook"
            );
        }
    }

    #[test]
    fn no_hook_carries_a_carriage_return() {
        // Passes trivially on a checkout whose sources are LF, and is the only
        // thing that catches a Windows checkout baking CRLF into the literals.
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell] {
            assert!(
                !generated(shell).contains('\r'),
                "{shell} script must be LF-only; a CRLF bash script fails on line 1"
            );
        }
    }

    #[test]
    fn bash_wrapper_is_registered_last() {
        let out = generated(Shell::Bash);
        assert!(out.contains("_rbx_dynamic() {"));
        // The last `complete -F` in the file is the one bash keeps.
        let last = out
            .rfind("complete -F ")
            .expect("bash script registers a completion function");
        assert!(
            out[last..].contains("_rbx_dynamic"),
            "the final registration must point at the wrapper, not at _rbx"
        );
    }

    #[test]
    fn zsh_routes_env_and_place_to_the_helpers() {
        let out = generated(Shell::Zsh);
        assert!(
            out.starts_with("#compdef rbx\n"),
            "the compdef tag must stay on the first line"
        );
        assert!(
            !out.contains(":ENV:_default"),
            "every --env action rewritten"
        );
        assert!(
            !out.contains(":PLACE:_default"),
            "every --place action rewritten"
        );
        assert!(out.contains(":ENV:_rbx_env_names"));
        assert!(out.contains(":PLACE:_rbx_place_names"));
        // The path to rbxplace.toml still completes as a path.
        assert!(out.contains(":PLACES:_files"));
        // Helpers defined before the trailing self-call, or they do not exist
        // yet on the invocation that autoloads the file.
        let defined = out.find("_rbx_env_names() {").expect("helper defined");
        let called = out.rfind("_rbx \"$@\"").expect("autoload self-call");
        assert!(defined < called);
    }

    #[test]
    fn fish_adds_value_completions_for_both_options() {
        let out = generated(Shell::Fish);
        assert!(out.contains(r#"complete -c rbx -s e -l env -r -f -a "(rbx env list --names"#));
        assert!(out.contains(r#"complete -c rbx -l place -r -f -a "(rbx env list --place-names"#));
    }

    #[test]
    fn powershell_hook_runs_before_the_static_switch() {
        let out = generated(Shell::PowerShell);
        let hook = out.find("$rbxPreviousToken").expect("hook present");
        let switch = out
            .find("$completions = @(switch ($command)")
            .expect("generated switch present");
        assert!(
            hook < switch,
            "the hook must return before the static candidate list is built"
        );
    }
}
