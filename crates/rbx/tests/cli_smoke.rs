//! Smoke tests on the `rbx` binary. Catches breakage when subcommands are
//! added, renamed, or refactored.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;

const SUBCOMMANDS: &[&str] = &[
    "shop", "place", "meta", "config", "secret", "apikey", "init", "env", "open", "download",
];

#[test]
fn help_lists_every_subcommand() {
    let mut cmd = Command::cargo_bin("rbx").unwrap();
    let assertion = cmd.arg("--help").assert().success();
    let stdout = String::from_utf8(assertion.get_output().stdout.clone()).unwrap();
    for sub in SUBCOMMANDS {
        assert!(
            stdout.contains(sub),
            "`rbx --help` should mention subcommand `{sub}`. Output was:\n{stdout}"
        );
    }
}

#[test]
fn version_flag_works() {
    Command::cargo_bin("rbx")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("rbx"));
}

#[test]
fn every_subcommand_has_help() {
    for sub in SUBCOMMANDS {
        Command::cargo_bin("rbx")
            .unwrap()
            .args([sub, "--help"])
            .assert()
            .success();
    }
}

#[test]
fn unknown_subcommand_fails_with_useful_message() {
    Command::cargo_bin("rbx")
        .unwrap()
        .arg("does-not-exist")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn global_flags_parse_before_subcommand() {
    // Parser check only. We pass --help to the subcommand so the binary
    // exits cleanly after rendering help (no real API call attempted) — what
    // matters is that the global flags placed before the subcommand are
    // accepted by the parser without error.
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["--api-key", "test-key", "--env", "dev", "shop", "--help"])
        .assert()
        .success();
}

#[test]
fn global_flags_parse_after_subcommand() {
    // Same as above but flags placed after the subcommand. Tests global=true
    // behaviour on every flag.
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["shop", "--api-key", "test-key", "--env", "dev", "--help"])
        .assert()
        .success();
}

#[test]
fn env_all_short_form_e_works() {
    // Parser check. `-e all` is the short form of `--env all`. The subcommand
    // help should still render cleanly.
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["-e", "all", "shop", "--help"])
        .assert()
        .success();
}

#[test]
fn completions_bash_emits_function() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_rbx()"));
}

#[test]
fn completions_zsh_emits_compdef() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef rbx"));
}

#[test]
fn completions_fish_emits_complete_commands() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c rbx"));
}

#[test]
fn completions_powershell_emits_registration() {
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["completions", "powershell"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Register-ArgumentCompleter"));
}

/// The four shells whose completion script carries the dynamic-value hook.
const HOOKED_SHELLS: &[&str] = &["bash", "zsh", "fish", "powershell"];

#[test]
fn completions_shell_out_for_env_and_place_values() {
    // The shell side of the hook cannot run in CI — there is no bash, zsh,
    // fish and pwsh here, and driving a real completion needs an interactive
    // terminal. What is testable is the contract between the two halves: the
    // script asks for these exact commands, and the commands answer with these
    // exact lines (pinned by the `env_list_*` tests below).
    for shell in HOOKED_SHELLS {
        Command::cargo_bin("rbx")
            .unwrap()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("rbx env list --names"))
            .stdout(predicate::str::contains("rbx env list --place-names"));
    }
}

#[test]
fn completions_no_dynamic_omits_the_hook() {
    for shell in HOOKED_SHELLS {
        Command::cargo_bin("rbx")
            .unwrap()
            .args(["completions", shell, "--no-dynamic"])
            .assert()
            .success()
            // Not a bare `rbx env list`: zsh's own generated helper functions
            // carry that phrase in their `_describe` tags.
            .stdout(predicate::str::contains("rbx env list --names").not())
            .stdout(predicate::str::contains("rbx env list --place-names").not());
    }
}

const PLACES: &str = "\
[dev]
universe_id = 100
[dev.places]
main = 1001
lobby = 1002

[prod]
universe_id = 200
[prod.places]
main = 2001
";

/// A `rbxplace.toml` in a directory of its own, so a test never reads the
/// repository's own file.
fn places_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rbxplace.toml");
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

#[test]
fn env_list_names_prints_one_env_per_line() {
    let (_dir, path) = places_file(PLACES);
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["env", "list", "--names", "--places"])
        .arg(&path)
        .assert()
        .success()
        .stdout("dev\nprod\n");
}

#[test]
fn env_list_place_names_unions_every_env_and_deduplicates() {
    // `main` is defined in both envs and must appear once: the completion menu
    // for `--place` lists roles, not occurrences.
    let (_dir, path) = places_file(PLACES);
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["env", "list", "--place-names", "--places"])
        .arg(&path)
        .assert()
        .success()
        .stdout("lobby\nmain\n");
}

#[test]
fn env_list_place_names_narrows_to_one_env() {
    let (_dir, path) = places_file(PLACES);
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["env", "list", "--place-names", "--env", "prod", "--places"])
        .arg(&path)
        .assert()
        .success()
        .stdout("main\n");
}

#[test]
fn env_list_place_names_is_empty_when_no_env_declares_a_place() {
    let (_dir, path) = places_file("[dev]\nuniverse_id = 100\n");
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["env", "list", "--place-names", "--places"])
        .arg(&path)
        .assert()
        .success()
        .stdout("");
}

#[test]
fn listers_write_nothing_to_stdout_without_a_places_file() {
    // The failure mode the hooks are built around. Completion runs wherever
    // the user is standing; outside a project both listers must fail with an
    // empty stdout, so redirecting stderr is all a hook has to do to stay
    // silent. If a diagnostic ever moved to stdout it would be offered as a
    // completion candidate.
    let dir = tempfile::tempdir().unwrap();
    for flag in ["--names", "--place-names"] {
        Command::cargo_bin("rbx")
            .unwrap()
            .args(["env", "list", flag])
            .current_dir(dir.path())
            .assert()
            .failure()
            .stdout("");
    }
}

#[test]
fn listers_write_nothing_to_stdout_for_a_malformed_places_file() {
    let (_dir, path) = places_file("[dev\nuniverse_id = ");
    for flag in ["--names", "--place-names"] {
        Command::cargo_bin("rbx")
            .unwrap()
            .args(["env", "list", flag, "--places"])
            .arg(&path)
            .assert()
            .failure()
            .stdout("");
    }
}

#[test]
fn place_names_and_names_are_mutually_exclusive() {
    let (_dir, path) = places_file(PLACES);
    Command::cargo_bin("rbx")
        .unwrap()
        .args(["env", "list", "--names", "--place-names", "--places"])
        .arg(&path)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}
