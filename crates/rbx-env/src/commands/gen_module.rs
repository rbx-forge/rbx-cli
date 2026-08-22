//! Export `rbxplace.toml` as a module your game code can import, so runtime
//! code can branch on the env it's running in without hardcoding ids.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use rbx_core::generated::{CheckReport, GeneratedFile};
use rbx_core::places::{Environment, PlacesFile};

/// One env, flattened and with its places sorted, ready to render.
///
/// Generation walks this instead of the raw maps: `PlacesFile` stores envs and
/// places in `HashMap`s, whose iteration order varies run to run. Emitting
/// straight from them produced a different file on every invocation, which
/// showed up as phantom diffs in whatever repo the module was committed to.
struct RenderEnv<'a> {
    /// The value game code matches on: `env = "..."` when set, else the
    /// section name.
    env_type: &'a str,
    universe_id: u64,
    places: Vec<(&'a str, u64)>,
}

pub fn run(places_path: &Path, out: Option<&str>, check: bool) -> Result<()> {
    let config = PlacesFile::load(places_path)?;
    let out_path = resolve_out(&config, out, places_path)?;
    let file = render(&config, &out_path)?;

    if !check {
        file.write()?;
        eprintln!("Generated: {}", file.path.display());
        return Ok(());
    }

    // Same bytes the write path would have produced, compared instead of
    // written: the point is to prove the committed module still follows
    // rbxplace.toml without needing credentials or a network.
    let mut report = CheckReport::new();
    report.check(&file)?;
    if !config.unknown.is_empty() {
        // The one cause where regenerating is the wrong move. An ignored key
        // is one this binary did not apply, so the render we just compared
        // against is the misreading, if the key was meant to keep an env out
        // of the module, the committed file is the correct side, and running
        // the fix widens `EnvironmentType` and breaks the exhaustive matches
        // the narrow type was protecting.
        report.note(format!(
            "{} key{} in {} {} ignored (listed above). If one of them was meant to change \
             what is generated, this check is reading the wrong inputs and the committed \
             file may be the correct one: regenerating would bake the misreading in. \
             Upgrade rbx, or fix the spelling, before running the fix.",
            config.unknown.len(),
            if config.unknown.len() == 1 { "" } else { "s" },
            places_path.display(),
            if config.unknown.len() == 1 {
                "was"
            } else {
                "were"
            },
        ));
    }
    report.finish(
        &places_path.display().to_string(),
        &format!(
            "rbx env gen-module{}",
            out.map(|o| format!(" --out {o}")).unwrap_or_default()
        ),
    )
}

/// Where to write: explicit `--out` wins, otherwise `[codegen].output` in
/// `rbxplace.toml`.
///
/// The config form is the one worth using in a hook or a CI job: a `--check`
/// spelled with a different path than the generator passes green while
/// verifying a file nobody consumes.
pub fn resolve_out(config: &PlacesFile, out: Option<&str>, places_path: &Path) -> Result<PathBuf> {
    if let Some(out) = out {
        return Ok(PathBuf::from(out));
    }

    let configured = config
        .codegen
        .as_ref()
        .and_then(|codegen| codegen.output.as_ref());

    match configured {
        // Relative to the file that declares it, so the command works from
        // anywhere in the repo: same rule as `rbx shop`'s codegen.output.
        Some(output) => Ok(places_path.parent().unwrap_or(Path::new(".")).join(output)),
        None => bail!(
            "No output path. Either pass --out <file>, or declare it once in {}:\n\n\
             \x20   [codegen]\n\
             \x20   output = \"src/shared/Envs.luau\"",
            places_path.display()
        ),
    }
}

/// Render the module for `out_path` without touching the filesystem. The
/// format follows the extension.
pub fn render(config: &PlacesFile, out_path: &Path) -> Result<GeneratedFile> {
    let envs = render_envs(config);
    if envs.is_empty() {
        // An empty module is not merely useless: the Luau and TypeScript
        // emitters would write `export type EnvironmentType = ` with nothing
        // after it, which does not parse.
        bail!(
            "No env left to generate from: every env in rbxplace.toml is marked              `codegen = false`. At least one env has to be visible to game code."
        );
    }

    let extension = out_path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("lua");

    let content = match extension {
        "lua" => generate_lua(&envs),
        "luau" => generate_luau(&envs),
        "json" => generate_json(&envs)?,
        "ts" => generate_typescript(&envs),
        ext => bail!("Unsupported format: .{}", ext),
    };

    Ok(GeneratedFile::new(out_path, content))
}

/// Envs game code should see, in name order. Anything marked
/// `codegen = false` is left out: it exists for tooling, and putting it in the
/// module would widen `EnvironmentType` and force game code to acknowledge an
/// env it never runs in.
fn render_envs(config: &PlacesFile) -> Vec<RenderEnv<'_>> {
    let mut names: Vec<&String> = config
        .environments
        .iter()
        .filter(|(_, env)| env.codegen)
        .map(|(name, _)| name)
        .collect();
    names.sort();

    names
        .into_iter()
        .map(|name| {
            let env: &Environment = &config.environments[name];
            let mut places: Vec<(&str, u64)> = env
                .places
                .iter()
                .map(|(place, id)| (place.as_str(), *id))
                .collect();
            places.sort_by(|a, b| a.0.cmp(b.0));

            RenderEnv {
                env_type: env.env.as_deref().unwrap_or(name.as_str()),
                universe_id: env.universe_id,
                places,
            }
        })
        .collect()
}

/// Header on every file this command emits, as the lines they occupy.
///
/// Kept as constants rather than repeated at each emitter: the `@generated`
/// token is what tooling matches on, so a copy that drifts stops being
/// recognized without anything failing.
///
/// Names the subcommand because `gen-module` is the only thing in `rbx env`
/// that writes a file. `rbx shop` names only the tool, since `sync` and
/// `codegen` both emit its folder.
const LUA_BANNER: [&str; 2] = [
    "-- This file is automatically @generated by rbx env gen-module.",
    "-- It is not intended for manual editing.",
];

const TS_BANNER: [&str; 2] = [
    "// This file is automatically @generated by rbx env gen-module.",
    "// It is not intended for manual editing.",
];

/// The banner as owned lines, ready to start a `Vec<String>` of output.
fn banner(lines: [&str; 2]) -> Vec<String> {
    lines.iter().map(|line| line.to_string()).collect()
}

fn generate_lua(envs: &[RenderEnv<'_>]) -> String {
    let mut lines = banner(LUA_BANNER);
    lines.push(String::new());
    lines.push("local envs = {".to_string());

    for env in envs {
        lines.push("  {".to_string());
        lines.push(format!("    env = \"{}\",", env.env_type));
        lines.push(format!("    universeId = {},", env.universe_id));

        lines.push("    placeIds = {".to_string());
        for (place_name, place_id) in &env.places {
            lines.push("      {".to_string());
            lines.push(format!("        name = \"{}\",", place_name));
            lines.push(format!("        id = {},", place_id));
            lines.push("      },".to_string());
        }
        lines.push("    }".to_string());

        lines.push("  },".to_string());
    }

    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("return envs".to_string());

    lines.join("\n")
}

fn generate_luau(envs: &[RenderEnv<'_>]) -> String {
    let mut lines = banner(LUA_BANNER);
    lines.push(String::new());

    let env_types: Vec<String> = envs
        .iter()
        .map(|env| format!("\"{}\"", env.env_type))
        .collect();

    lines.push(format!(
        "export type EnvironmentType = {}",
        env_types.join(" | ")
    ));
    lines.push(String::new());

    lines.push("export type EnvironmentInfo = {".to_string());
    lines.push("  env: EnvironmentType,".to_string());
    lines.push("  universeId: number,".to_string());
    lines.push("  placeIds: { { name: string, id: number } },".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    lines.push("local envs: { EnvironmentInfo } = {".to_string());

    for env in envs {
        lines.push("  {".to_string());
        lines.push(format!("    env = \"{}\",", env.env_type));
        lines.push(format!("    universeId = {},", env.universe_id));

        lines.push("    placeIds = {".to_string());
        for (place_name, place_id) in &env.places {
            lines.push("      {".to_string());
            lines.push(format!("        name = \"{}\",", place_name));
            lines.push(format!("        id = {},", place_id));
            lines.push("      },".to_string());
        }
        lines.push("    }".to_string());

        lines.push("  },".to_string());
    }

    lines.push("}".to_string());
    lines.push(String::new());
    lines.push("return envs".to_string());

    lines.join("\n")
}

fn generate_json(envs: &[RenderEnv<'_>]) -> Result<String> {
    let rendered: Vec<serde_json::Value> = envs
        .iter()
        .map(|env| {
            let place_ids: Vec<serde_json::Value> = env
                .places
                .iter()
                .map(|(name, id)| serde_json::json!({ "name": name, "id": id }))
                .collect();

            serde_json::json!({
                "env": env.env_type,
                "universeId": env.universe_id,
                "placeIds": place_ids,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&rendered)?)
}

fn generate_typescript(envs: &[RenderEnv<'_>]) -> String {
    let mut lines = banner(TS_BANNER);
    lines.push(String::new());

    let env_names: Vec<String> = envs
        .iter()
        .map(|env| format!("\"{}\"", env.env_type))
        .collect();

    lines.push(format!(
        "export type EnvironmentType = {};",
        env_names.join(" | ")
    ));
    lines.push(String::new());

    lines.push("export interface PlaceInfo {".to_string());
    lines.push("  name: string;".to_string());
    lines.push("  id: number;".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    lines.push("export interface EnvironmentInfo {".to_string());
    lines.push("  env: EnvironmentType;".to_string());
    lines.push("  universeId: number;".to_string());
    lines.push("  placeIds: PlaceInfo[];".to_string());
    lines.push("}".to_string());
    lines.push(String::new());

    lines.push("export const ENVIRONMENTS: EnvironmentInfo[] = [".to_string());

    for env in envs {
        lines.push("  {".to_string());
        lines.push(format!("    env: \"{}\",", env.env_type));
        lines.push(format!("    universeId: {},", env.universe_id));

        lines.push("    placeIds: [".to_string());
        for (place_name, place_id) in &env.places {
            lines.push("      {".to_string());
            lines.push(format!("        name: \"{}\",", place_name));
            lines.push(format!("        id: {},", place_id));
            lines.push("      },".to_string());
        }
        lines.push("    ]".to_string());

        lines.push("  },".to_string());
    }

    lines.push("];".to_string());

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> PlacesFile {
        toml::from_str(toml).unwrap()
    }

    const SAMPLE: &str = r#"
[prod]
universe_id = 200
[prod.places]
main = 2001
arena = 2002

[dev]
universe_id = 100
env = "development"
[dev.places]
main = 1001
"#;

    #[test]
    fn envs_and_places_come_out_sorted() {
        // The whole point of RenderEnv: HashMap order must not leak into the
        // generated file, or every regeneration is a spurious diff.
        let parsed = config(SAMPLE);
        let envs = render_envs(&parsed);
        assert_eq!(envs.len(), 2);
        // "dev" sorts before "prod" even though it is declared second.
        assert_eq!(envs[0].env_type, "development");
        assert_eq!(envs[1].env_type, "prod");
        assert_eq!(envs[1].places, vec![("arena", 2002), ("main", 2001)]);
    }

    #[test]
    fn generation_is_stable_across_runs() {
        let parsed = config(SAMPLE);
        let first = generate_luau(&render_envs(&parsed));
        for _ in 0..20 {
            let again = generate_luau(&render_envs(&config(SAMPLE)));
            assert_eq!(
                first, again,
                "output must not depend on map iteration order"
            );
        }
    }

    #[test]
    fn env_override_wins_over_the_section_name() {
        let parsed = config(SAMPLE);
        let envs = render_envs(&parsed);
        // [dev] declares env = "development", so that is what game code matches.
        assert_eq!(envs[0].env_type, "development");
    }

    /// `.lua` and `.luau` differ only in the type declarations, which is easy
    /// to lose track of when editing one emitter. The snapshots below show both
    /// outputs; this states which way round it goes.
    #[test]
    fn lua_stays_untyped_where_luau_is_typed() {
        let parsed = config(SAMPLE);
        let envs = render_envs(&parsed);
        assert!(!generate_lua(&envs).contains("export type"));
        assert!(generate_luau(&envs).contains("export type"));
    }

    // ── whole-module snapshots ──
    //
    // Every emitted format, in full, from one fixture. These are a contract
    // with the user's game code: `EnvironmentType` narrows the exhaustive
    // matches downstream, so a union that quietly gains or loses a member
    // breaks code that compiled yesterday. Fragment assertions could not see
    // that; a `contains()` on one line says nothing about the twenty around
    // it.
    //
    // Run `cargo insta review` to accept an intended change.

    /// Two envs game code sees: one renamed with `env = "..."`, one with two
    /// places, plus a third marked `codegen = false`, which must appear in
    /// none of the four outputs.
    const SNAPSHOT_FIXTURE: &str = r#"
[prod]
universe_id = 200
[prod.places]
main = 2001
arena = 2002

[dev]
universe_id = 100
env = "development"
[dev.places]
main = 1001
lobby = 1002

[ci]
universe_id = 555
codegen = false
[ci.places]
main = 5001
"#;

    #[test]
    fn luau_module() {
        let parsed = config(SNAPSHOT_FIXTURE);
        let envs = render_envs(&parsed);
        insta::assert_snapshot!("luau", generate_luau(&envs));
    }

    #[test]
    fn lua_module() {
        let parsed = config(SNAPSHOT_FIXTURE);
        let envs = render_envs(&parsed);
        insta::assert_snapshot!("lua", generate_lua(&envs));
    }

    #[test]
    fn json_module() {
        let parsed = config(SNAPSHOT_FIXTURE);
        let envs = render_envs(&parsed);
        insta::assert_snapshot!("json", generate_json(&envs).unwrap());
    }

    #[test]
    fn typescript_module() {
        let parsed = config(SNAPSHOT_FIXTURE);
        let envs = render_envs(&parsed);
        insta::assert_snapshot!("typescript", generate_typescript(&envs));
    }

    #[test]
    fn every_commented_format_carries_the_generated_banner() {
        // The marker is what tooling matches on, so it is worth asserting
        // rather than trusting three separate emitters to keep saying it.
        let parsed = config(SAMPLE);
        let envs = render_envs(&parsed);
        for (label, out) in [
            ("lua", generate_lua(&envs)),
            ("luau", generate_luau(&envs)),
            ("ts", generate_typescript(&envs)),
        ] {
            assert!(
                out.contains("This file is automatically @generated by rbx env gen-module."),
                "{label} is missing the generated marker:
{out}"
            );
            assert!(
                out.contains("It is not intended for manual editing."),
                "{label} is missing the second banner line"
            );
        }
    }

    #[test]
    fn the_banner_uses_each_language_s_comment_syntax() {
        let parsed = config(SAMPLE);
        let envs = render_envs(&parsed);
        assert!(generate_luau(&envs).starts_with("-- This file"));
        assert!(generate_typescript(&envs).starts_with("// This file"));
    }

    #[test]
    fn json_carries_no_banner_because_json_has_no_comments() {
        // Deliberate, not an oversight: the one emitted format that cannot
        // carry the marker. Asserted so nobody "fixes" it into invalid JSON.
        let out = generate_json(&render_envs(&config(SAMPLE))).unwrap();
        assert!(!out.contains("@generated"));
        serde_json::from_str::<serde_json::Value>(&out).expect("must stay valid JSON");
    }

    #[test]
    fn check_passes_right_after_generating_and_fails_once_the_file_is_edited() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, SAMPLE).unwrap();
        let out = dir.path().join("Envs.luau");
        let out_str = out.to_str().unwrap();

        run(&places, Some(out_str), false).unwrap();
        run(&places, Some(out_str), true).expect("a just-generated file must pass its own check");

        std::fs::write(&out, "-- hand-edited\nreturn {}\n").unwrap();
        assert!(
            run(&places, Some(out_str), true).is_err(),
            "a hand-edited module must fail the check"
        );
    }

    #[test]
    fn the_output_path_can_come_from_the_toml_instead_of_out() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            format!(
                "[codegen]
output = \"Envs.luau\"
{SAMPLE}"
            ),
        )
        .unwrap();

        // No --out anywhere: generate and check must agree on the path by
        // reading it from the same place.
        run(&places, None, false).unwrap();
        assert!(dir.path().join("Envs.luau").exists());
        run(&places, None, true).unwrap();
    }

    #[test]
    fn the_configured_output_is_relative_to_the_toml_not_the_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("config");
        std::fs::create_dir(&nested).unwrap();
        let places = nested.join("rbxplace.toml");
        std::fs::write(
            &places,
            format!(
                "[codegen]
output = \"out/Envs.luau\"
{SAMPLE}"
            ),
        )
        .unwrap();

        run(&places, None, false).unwrap();
        assert!(nested.join("out/Envs.luau").exists());
    }

    #[test]
    fn out_wins_over_the_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            format!(
                "[codegen]
output = \"Configured.luau\"
{SAMPLE}"
            ),
        )
        .unwrap();

        let explicit = dir.path().join("Explicit.luau");
        run(&places, Some(explicit.to_str().unwrap()), false).unwrap();
        assert!(explicit.exists());
        assert!(!dir.path().join("Configured.luau").exists());
    }

    #[test]
    fn no_out_and_no_config_names_both_ways_to_fix_it() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, SAMPLE).unwrap();

        let err = run(&places, None, false).unwrap_err().to_string();
        assert!(err.contains("--out"), "got: {err}");
        assert!(err.contains("[codegen]"), "got: {err}");
    }

    #[test]
    fn check_fails_when_the_module_was_never_generated() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, SAMPLE).unwrap();
        let out = dir.path().join("Envs.luau");
        assert!(run(&places, Some(out.to_str().unwrap()), true).is_err());
    }

    #[test]
    fn a_tooling_env_is_left_out_of_the_module() {
        let parsed = config(
            "[dev]
universe_id = 100
[ci]
universe_id = 555
codegen = false
",
        );
        let envs = render_envs(&parsed);
        assert_eq!(envs.len(), 1, "ci must not reach game code");
        assert_eq!(envs[0].env_type, "dev");

        // The point of the flag: the union does not widen.
        let out = generate_luau(&envs);
        assert!(
            out.contains("export type EnvironmentType = \"dev\""),
            "got:
{out}"
        );
        assert!(!out.contains("555"));
    }

    #[test]
    fn check_warns_that_regenerating_is_wrong_when_a_key_was_ignored() {
        // The reported failure, reproduced with a key no release will have:
        // a v0.7.0 binary swallowed `codegen = false`, generated the env
        // anyway, and told the reader to commit that, which widens
        // `EnvironmentType` and breaks the exhaustive matches downstream.
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        let out = dir.path().join("Envs.luau");
        let out_str = out.to_str().unwrap();

        std::fs::write(&places, "[dev]\nuniverse_id = 100\n").unwrap();
        run(&places, Some(out_str), false).unwrap();

        // Now the file gains a key this binary does not read, next to an env
        // the committed module does not contain.
        std::fs::write(
            &places,
            "[dev]\nuniverse_id = 100\n[ci]\nuniverse_id = 555\ncodegen_from_the_future = false\n",
        )
        .unwrap();

        let err = run(&places, Some(out_str), true).unwrap_err().to_string();
        assert!(err.contains("ignored"), "got: {err}");
        assert!(
            err.contains("unless one of the following applies"),
            "the fix must stop being the single stated remedy: {err}"
        );
    }

    #[test]
    fn check_states_the_fix_plainly_when_every_key_was_understood() {
        // The qualifier is conditional on purpose: hedging every drift report
        // would bury the case where regenerating really is the whole answer.
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(&places, SAMPLE).unwrap();
        let out = dir.path().join("Envs.luau");
        let out_str = out.to_str().unwrap();

        run(&places, Some(out_str), false).unwrap();
        std::fs::write(&out, "-- hand-edited\nreturn {}\n").unwrap();

        let err = run(&places, Some(out_str), true).unwrap_err().to_string();
        assert!(err.contains("commit the result."), "got: {err}");
        assert!(!err.contains("unless one of the following"), "got: {err}");
    }

    #[test]
    fn hiding_every_env_is_refused_rather_than_emitting_a_broken_union() {
        let dir = tempfile::tempdir().unwrap();
        let places = dir.path().join("rbxplace.toml");
        std::fs::write(
            &places,
            "[ci]
universe_id = 555
codegen = false
",
        )
        .unwrap();
        let err = run(&places, Some("x.luau"), false).unwrap_err().to_string();
        // `export type EnvironmentType = ` with nothing after it does not parse.
        assert!(err.contains("codegen = false"), "got: {err}");
    }

    #[test]
    fn an_env_without_places_still_renders() {
        let out = generate_luau(&render_envs(&config("[dev]\nuniverse_id = 100\n")));
        assert!(out.contains("placeIds = {\n    }"), "got:\n{out}");
    }
}
