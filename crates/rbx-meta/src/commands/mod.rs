pub mod check;
pub mod init;
pub mod pull;
pub mod sync;

use anyhow::{bail, Result};

use crate::lockfile::EnvLock;

/// What a lockfile section is allowed to be re-pointed at.
///
/// `sync` and `pull` both write `[envs.<name>]` from whatever `--env` and
/// `--place` resolved to. Neither may do that when the section already
/// describes something else, so the check lives here rather than twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repoint {
    /// Refuse a universe change as well. `sync` writes to Roblox and computes
    /// what to send by diffing against this section, so a section describing
    /// another experience makes every decision in the run wrong.
    Nothing,
    /// Allow a universe change, refuse a place change. Repointing an env at a
    /// new universe and pulling is how you adopt it, and `sync` already
    /// refuses that case and tells you to delete the section — closing the
    /// door here too would leave no way through.
    Universe,
}

/// Refuse to silently re-point an env's lockfile section.
///
/// The place check is the one that catches a case nothing else can see. The
/// section is keyed by env and holds a single `place_id`, while `name`,
/// `description` and `server_size` are place-level fields. So an env with two
/// places, synced or pulled once per place, would rewrite the section in place
/// and leave it recording one place's metadata under the other's id. Every
/// later diff is then computed against the wrong baseline, and a field can be
/// skipped because the *other* place already had that value.
///
/// Refusing is the honest answer while the format has one slot for it. Keying
/// the section by `(env, place)` is the real fix and is a format change.
pub fn ensure_not_repointed(
    env: &str,
    lock: &EnvLock,
    universe_id: u64,
    place_id: u64,
    allow: Repoint,
) -> Result<()> {
    // Zero is "this section has never been written", not a real id.
    if allow == Repoint::Nothing && lock.universe_id != 0 && lock.universe_id != universe_id {
        bail!(
            "Lockfile env '{env}' tracks universe_id {} but the resolved target is {universe_id}. \
             Delete the [envs.{env}] section if intentional.",
            lock.universe_id
        );
    }

    if lock.place_id != 0 && lock.place_id != place_id {
        bail!(
            "Lockfile env '{env}' tracks place_id {} but the resolved target is {place_id}. \
             This env has more than one place and the lockfile holds one, so writing the second \
             would record its metadata under the first one's id. Use one place per env, or \
             delete the [envs.{env}] section if you meant to move it.",
            lock.place_id
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock(universe_id: u64, place_id: u64) -> EnvLock {
        EnvLock {
            universe_id,
            place_id,
            ..Default::default()
        }
    }

    #[test]
    fn an_unwritten_section_accepts_anything() {
        // Zero on both is a section that has never been written, which every
        // first run has. It must not read as a mismatch against id 0.
        assert!(ensure_not_repointed("dev", &lock(0, 0), 11, 22, Repoint::Nothing).is_ok());
        assert!(ensure_not_repointed("dev", &lock(0, 0), 11, 22, Repoint::Universe).is_ok());
    }

    #[test]
    fn the_same_target_is_not_a_repoint() {
        assert!(ensure_not_repointed("dev", &lock(11, 22), 11, 22, Repoint::Nothing).is_ok());
    }

    #[test]
    fn a_second_place_in_one_env_is_refused_by_both_callers() {
        // The bug this exists for: `--place lobby` then `--place main` against
        // one env, which used to rewrite the section rather than refuse.
        for allow in [Repoint::Nothing, Repoint::Universe] {
            let err = ensure_not_repointed("prod", &lock(11, 22), 11, 33, allow)
                .expect_err("a different place must be refused");
            let text = format!("{err:#}");
            assert!(
                text.contains("place_id 22"),
                "names what is tracked: {text}"
            );
            assert!(text.contains("33"), "names what was resolved: {text}");
            assert!(
                text.contains("[envs.prod]"),
                "names the section to delete: {text}"
            );
        }
    }

    #[test]
    fn a_different_universe_is_refused_for_sync_and_allowed_for_pull() {
        // sync diffs against this section to decide what to send, so a section
        // describing another experience poisons the whole run.
        assert!(ensure_not_repointed("dev", &lock(11, 22), 99, 22, Repoint::Nothing).is_err());
        // pull is how an adopted universe gets written down, and sync's own
        // message sends you here. Refusing would leave no way through.
        assert!(ensure_not_repointed("dev", &lock(11, 22), 99, 22, Repoint::Universe).is_ok());
    }
}
