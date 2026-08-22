pub mod catalog;
pub mod create;
pub mod delete;
pub mod introspect;
pub mod list;
pub mod permissions;
pub mod prune;
pub mod regenerate;
pub mod remote;
pub mod resolve;
pub mod scopes;
pub mod status;
pub mod update;

use anyhow::{bail, Result};

use crate::api::RbxApiKeyClient;
use crate::config;
use rbx_core::GlobalFlags;

/// What a command says when the name it was handed is not a key in
/// `rbxapikey.toml`.
///
/// One wording for both `create` and `update`, because the interesting case is
/// the same in each and it is not the typo: a declaration that fans out into
/// one key per env group is not itself a key, so `rbx apikey create deploy`
/// matches nothing while `[keys.deploy]` sits there in the file. Saying only
/// "not in rbxapikey.toml" is true and reads like the edit was never saved.
pub fn missing_key_note(cfg: &config::Config, name: &str) -> String {
    let generated = config::keys_from_declaration(cfg, name);
    if generated.is_empty() {
        return format!("skipping \"{}\": not in {}", name, config::FILE);
    }
    format!(
        "skipping \"{}\": it declares one key per env group rather than a key of its own - \
         name one of {}",
        name,
        generated.join(", ")
    )
}

/// Build the HTTP client on whatever cookie the global flags resolve to.
///
/// No `.or_else` fallback. There used to be one, onto a second lookup that did
/// not honour `--no-auto-cookie`, which is how the flag came to be ignored by
/// every subcommand here. `resolve_cookie` now reads `RBXAPIKEY_COOKIE` itself,
/// so this has nothing left to add.
pub fn make_client(global: &GlobalFlags) -> RbxApiKeyClient {
    RbxApiKeyClient::new(global.resolve_cookie())
}

pub fn require_no_collision(all: bool, name: Option<&str>) -> Result<()> {
    if all && name.is_some() {
        bail!("--all and <key> are mutually exclusive");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(no_auto_cookie: bool) -> GlobalFlags {
        GlobalFlags {
            api_key: None,
            cookie: None,
            no_auto_cookie,
            auto_cookie: false,
            env: None,
            place: None,
            places: "rbxplace.toml".into(),
            universe_id: None,
            place_id: Vec::new(),
        }
    }

    /// #20, from this side. `resolve_cookie` owns the whole decision, so the
    /// only way this crate can get it wrong again is by adding a source of its
    /// own, which is exactly what the `.or_else` here used to be.
    ///
    /// Asserting the two agree, rather than asserting a particular cookie,
    /// keeps this true on a machine with Studio signed in and in CI without.
    /// What `resolve_cookie` decides is `rbx-core`'s business and is tested
    /// there against a seam; what this owns is adding nothing to it.
    #[test]
    fn make_client_takes_the_resolved_cookie_and_adds_nothing() {
        for no_auto_cookie in [false, true] {
            let global = flags(no_auto_cookie);
            assert_eq!(
                make_client(&global).cookie(),
                global.resolve_cookie().as_deref(),
                "make_client must not resolve a cookie of its own (--no-auto-cookie: \
                 {no_auto_cookie})"
            );
        }
    }

    /// The name of a fan-out declaration matches no key, and the useful half
    /// of the answer is which keys it did produce.
    #[test]
    fn naming_a_declaration_instead_of_a_key_lists_the_keys_it_made() {
        let cfg = config::load_from(std::path::Path::new(&write_config(
            "[keys.deploy]\nscopes = [\"universe:read\"]\n\n\
             [keys.deploy.envs]\nci = [\"dev\"]\nprod = [\"prod\"]\n",
        )))
        .unwrap();

        let note = missing_key_note(&cfg, "deploy");
        assert!(note.contains("deploy_ci"), "{note}");
        assert!(note.contains("deploy_prod"), "{note}");

        // A name that is nobody's declaration keeps the plain answer.
        let note = missing_key_note(&cfg, "nosuchkey");
        assert!(note.contains("not in rbxapikey.toml"), "{note}");
        // And a real key never reaches this message at all.
        assert!(config::get(&cfg, "deploy_ci").is_some());
    }

    fn write_config(text: &str) -> String {
        let dir = std::env::temp_dir().join(format!("rbxapikey_note_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(config::FILE);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn an_explicit_cookie_reaches_the_client() {
        let mut global = flags(false);
        global.cookie = Some("explicit".into());
        assert_eq!(make_client(&global).cookie(), Some("explicit"));
    }
}
