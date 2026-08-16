//! `--help` must never print the value of a credential environment variable.
//!
//! `GlobalFlags` carries `hide_env_values = true` on `--api-key` and
//! `--cookie`, and a comment above them says it is load bearing rather than
//! cosmetic: without it clap renders `[env: RBX_API_KEY=<the actual key>]` into
//! every help page. A comment is not a guarantee. Someone adding the next
//! credential flag copies the line above it, and the day they copy one that
//! predates the comment, the key lands in whatever pasted a help page: a CI
//! log, an issue report, a screenshot.
//!
//! Help is the worst surface for this because it is the output people share
//! most freely. Nobody redacts a `--help` before pasting it into a bug report.

#![allow(clippy::unwrap_used)]

use assert_cmd::prelude::*;
use std::process::Command;

/// Values that cannot occur by accident, so a match is always a leak.
const API_KEY: &str = "RBX-HELP-LEAK-CANARY-4e91c7";
const COOKIE: &str = "ROBLOSECURITY-HELP-LEAK-CANARY-b3d82f";

/// Every help surface a user is likely to paste somewhere.
///
/// The subcommands are the ones whose flags are credential-adjacent, plus the
/// bare root: `--api-key` and `--cookie` are `global = true`, so they render on
/// every subcommand's help as well as the root's.
const HELP_INVOCATIONS: &[&[&str]] = &[
    &["--help"],
    &["apikey", "--help"],
    &["apikey", "status", "--help"],
    &["meta", "--help"],
    &["shop", "pull", "--help"],
    &["check", "--help"],
];

fn help_output(args: &[&str]) -> String {
    let output = Command::cargo_bin("rbx")
        .unwrap()
        .env("RBX_API_KEY", API_KEY)
        .env("RBX_COOKIE", COOKIE)
        .args(args)
        .assert()
        .success()
        .get_output()
        .clone();

    let mut both = String::from_utf8_lossy(&output.stdout).into_owned();
    both.push_str(&String::from_utf8_lossy(&output.stderr));
    both
}

/// The whole point of `hide_env_values`, asserted rather than commented.
#[test]
fn help_never_prints_the_value_of_a_credential_environment_variable() {
    for args in HELP_INVOCATIONS {
        let text = help_output(args);

        for (name, value) in [("RBX_API_KEY", API_KEY), ("RBX_COOKIE", COOKIE)] {
            assert!(
                !text.contains(value),
                "`rbx {}` printed the value of {name} into its help. \
                 That help gets pasted into CI logs and issue reports. \
                 The flag needs `hide_env_values = true`.\n{text}",
                args.join(" "),
            );
        }
    }
}

/// The variable *names* must stay visible: they are how a reader learns the
/// flag can be set from the environment at all. Hiding the value is the point,
/// hiding the name would make the feature undiscoverable, and a test that
/// only forbade things could be satisfied by removing the `env` attribute
/// altogether.
#[test]
fn help_still_names_the_environment_variables_it_reads() {
    let text = help_output(&["--help"]);

    for name in ["RBX_API_KEY", "RBX_COOKIE"] {
        assert!(
            text.contains(name),
            "`rbx --help` no longer mentions {name}, so nobody can discover it.\n{text}"
        );
    }
}
