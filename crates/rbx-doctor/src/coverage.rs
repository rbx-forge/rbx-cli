//! Step 4: what the key can and cannot run, for the tools this repo configures.
//!
//! # Presence, not parsing
//!
//! The unit of detection is a config file existing. `doctor` does not parse
//! `rbxshop.toml` to find out whether it declares badges, and does not link
//! against `rbx-shop` to ask.
//!
//! That is a deliberate ceiling on what this can claim, and it buys the thing
//! that matters: `rbx-doctor` depends on `rbx-core` and `rbx-apikey` and stops
//! there. Parsing each tool's config properly means depending on every domain
//! crate in the workspace, which turns a diagnostic command into the one crate
//! that has to be rebuilt whenever anything changes. #50 (`rbx check`) will
//! face the same question at a larger scale, and the answer that survives there
//! is the one worth adopting here.
//!
//! The cost is over-reporting: a repo with an `rbxshop.toml` that only declares
//! game passes is told it cannot manage badges. The line says which scope is
//! missing for which operation, so a reader who does not use badges can see
//! that it does not apply to them, which is a far better failure than silence
//! about a scope they do need.
//!
//! # Where the requirements come from
//!
//! Each tool's `docs/<tool>.md` carries a "Required API scopes" table, and this
//! is a transcription of those tables. They are the documented contract, so a
//! divergence here is a bug in one of the two places rather than a judgement
//! call.

use std::path::Path;

/// One thing a tool can do, and the scopes needed to do it.
#[derive(Debug, Clone, Copy)]
pub struct Operation {
    /// What the user would call this, phrased as the command they would run.
    pub what: &'static str,
    /// Every `(scope_type, operation)` the call needs. All of them, not any:
    /// a key holding half of them fails on the other half.
    pub scopes: &'static [(&'static str, &'static str)],
}

/// A config file, the tool that reads it, and what that tool needs.
#[derive(Debug, Clone, Copy)]
pub struct ToolRequirements {
    pub config_file: &'static str,
    pub tool: &'static str,
    pub operations: &'static [Operation],
}

/// Transcribed from the "Required API scopes" table in each tool's doc.
pub const REQUIREMENTS: &[ToolRequirements] = &[
    ToolRequirements {
        config_file: "rbxplace.toml",
        tool: "rbx place",
        operations: &[
            Operation {
                what: "rbx place upload / promote",
                scopes: &[("universe-places", "write")],
            },
            Operation {
                what: "rbx place download",
                scopes: &[("legacy-asset", "manage")],
            },
            Operation {
                what: "rbx place versions",
                scopes: &[("asset", "read")],
            },
            Operation {
                what: "rbx place rollback",
                scopes: &[("asset", "write")],
            },
            Operation {
                what: "rbx place places",
                scopes: &[("universe", "read")],
            },
        ],
    },
    ToolRequirements {
        config_file: "rbxmeta.toml",
        tool: "rbx meta",
        operations: &[
            Operation {
                what: "rbx meta sync (universe fields)",
                scopes: &[("universe", "read"), ("universe", "write")],
            },
            Operation {
                what: "rbx meta sync (place fields)",
                scopes: &[("universe.place", "read"), ("universe.place", "write")],
            },
            Operation {
                what: "rbx meta sync (icons & thumbnails)",
                scopes: &[("universe.image", "read"), ("universe.image", "write")],
            },
        ],
    },
    ToolRequirements {
        config_file: "rbxconfig.toml",
        tool: "rbx config",
        operations: &[
            Operation {
                what: "rbx config get / list / check / pull",
                scopes: &[("universe", "read")],
            },
            Operation {
                what: "rbx config sync / rollback",
                scopes: &[("universe", "write")],
            },
        ],
    },
    ToolRequirements {
        config_file: "rbxshop.toml",
        tool: "rbx shop",
        operations: &[
            Operation {
                what: "rbx shop sync (game passes)",
                scopes: &[("game-pass", "read"), ("game-pass", "write")],
            },
            Operation {
                what: "rbx shop sync (developer products)",
                scopes: &[
                    ("developer-product", "read"),
                    ("developer-product", "write"),
                ],
            },
            Operation {
                what: "rbx shop sync (badges)",
                // `read` is not among them, though a badge listing plainly
                // reads: Roblox rejects a key declaring
                // `legacy-universe.badge:read` with `400 InvalidScopes`,
                // measured against the live API on 2026-08-16. Asking for it
                // here sent a reader to add a scope that cannot be created,
                // and the embedded catalog has said so all along: it lists
                // two operations for this type, not three.
                scopes: &[
                    ("legacy-universe.badge", "write"),
                    ("legacy-universe.badge", "manage-and-spend-robux"),
                ],
            },
            Operation {
                what: "rbx shop sync (icons)",
                scopes: &[("legacy-asset", "manage")],
            },
        ],
    },
];

/// The requirements for the config files actually sitting in `dir`.
pub fn present_in(dir: &Path) -> Vec<&'static ToolRequirements> {
    REQUIREMENTS
        .iter()
        .filter(|r| dir.join(r.config_file).is_file())
        .collect()
}

/// Render `[("universe", "read"), ("universe", "write")]` the way the config
/// file spells it, so a reader can paste it into `rbxapikey.toml`.
///
/// Operations on the same scope type are folded into one entry
/// (`universe:read,write`) because that is the form `rbxapikey.toml` accepts;
/// listing them separately would produce something that does not parse.
pub fn spell_scopes(scopes: &[(&str, &str)]) -> String {
    let mut grouped: Vec<(&str, Vec<&str>)> = Vec::new();
    for (scope_type, op) in scopes {
        match grouped.iter_mut().find(|(t, _)| t == scope_type) {
            Some((_, ops)) => ops.push(op),
            None => grouped.push((scope_type, vec![op])),
        }
    }
    grouped
        .into_iter()
        .map(|(t, ops)| format!("{}:{}", t, ops.join(",")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_config_files_actually_there_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("rbxshop.toml"), "").unwrap();

        let found = present_in(dir.path());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].config_file, "rbxshop.toml");
    }

    #[test]
    fn an_empty_directory_requires_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(present_in(dir.path()).is_empty());
    }

    /// A directory named `rbxplace.toml` is not a config file, and treating it
    /// as one would report requirements for a tool that cannot run here.
    #[test]
    fn a_directory_with_a_config_name_is_not_a_config_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("rbxplace.toml")).unwrap();
        assert!(present_in(dir.path()).is_empty());
    }

    #[test]
    fn operations_on_one_scope_type_are_folded_into_one_entry() {
        assert_eq!(
            spell_scopes(&[("universe", "read"), ("universe", "write")]),
            "universe:read,write"
        );
    }

    #[test]
    fn different_scope_types_stay_separate() {
        assert_eq!(
            spell_scopes(&[("universe", "read"), ("asset", "read")]),
            "universe:read asset:read"
        );
    }

    /// Every entry has to be usable in `rbxapikey.toml`: a `scopes = [...]`
    /// element is `type:op[,op]`, so an empty half on either side of the colon
    /// would be a line the tool tells people to paste and then rejects.
    #[test]
    fn every_declared_requirement_spells_out_to_a_usable_scope_string() {
        for tool in REQUIREMENTS {
            for op in tool.operations {
                assert!(!op.scopes.is_empty(), "{} declares no scopes", op.what);
                for (scope_type, operation) in op.scopes {
                    assert!(!scope_type.is_empty(), "{}", op.what);
                    assert!(!operation.is_empty(), "{}", op.what);
                    assert!(!scope_type.contains(':'), "{scope_type} has a colon");
                    assert!(!operation.contains(','), "{operation} has a comma");
                }
            }
        }
    }

    /// Scopes the docs require that the bundled catalog does not corroborate.
    ///
    /// The catalog is generated from Roblox's own `openapi.json`; these tables
    /// are hand-written. Where the two disagree, one of them is wrong, and
    /// which one is not this crate's call to make: `rbx apikey` accepts an
    /// unknown scope with a warning rather than refusing it, precisely because
    /// Roblox's spec has been observed to lag what the API accepts.
    ///
    /// So the divergences are recorded rather than resolved. Listing them here
    /// keeps [`every_required_scope_is_a_known_one_or_a_recorded_divergence`]
    /// able to catch an actual typo, and makes the set shrink visibly: when
    /// Roblox publishes one of these, that test fails and the entry gets
    /// deleted.
    ///
    /// - `universe.image:read` / `:write`: `docs/meta.md` requires them for
    ///   icons and thumbnails and notes the calls go to
    ///   `legacy-game-internationalization` endpoints. No `universe.image`
    ///   scope of any kind is in the catalog.
    /// - `legacy-universe.badge:read`: the type is in the catalog, with
    ///   `write` and `manage-and-spend-robux` but no `read`.
    const UNCORROBORATED: &[(&str, &str)] = &[
        ("universe.image", "read"),
        ("universe.image", "write"),
        ("legacy-universe.badge", "read"),
    ];

    fn catalog_corroborates(scope_type: &str, operation: &str) -> bool {
        let lookup = rbx_apikey::scope_catalog::lookup(scope_type);
        lookup.known
            && rbx_apikey::scope_catalog::unknown_operations(scope_type, &[operation.to_string()])
                .is_empty()
    }

    #[test]
    fn every_required_scope_is_a_known_one_or_a_recorded_divergence() {
        for tool in REQUIREMENTS {
            for op in tool.operations {
                for (scope_type, operation) in op.scopes {
                    assert!(
                        catalog_corroborates(scope_type, operation)
                            || UNCORROBORATED.contains(&(scope_type, operation)),
                        "{scope_type}:{operation} (required by {}) is neither in the scope \
                         catalog nor a recorded divergence: typo, or a scope worth recording",
                        op.what
                    );
                }
            }
        }
    }

    /// The other direction, so the list shrinks on its own. A divergence that
    /// the catalog has caught up with is no longer a divergence, and leaving it
    /// listed would let a real typo hide behind it.
    #[test]
    fn no_recorded_divergence_has_quietly_been_resolved() {
        for (scope_type, operation) in UNCORROBORATED {
            assert!(
                !catalog_corroborates(scope_type, operation),
                "{scope_type}:{operation} is in the catalog now: drop it from UNCORROBORATED"
            );
        }
    }
}
