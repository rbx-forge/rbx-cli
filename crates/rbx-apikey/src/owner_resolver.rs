//! Resolve and cache env → (universe_id, owner) mappings.
//!
//! In schema v4 the lockfile stores a single `[envs.<name>]` table that
//! every key shares. This module keeps that table in sync with
//! `rbxplace.toml` + (optionally) the Roblox games API:
//!
//! - `universe_id` is always taken from `rbxplace.toml` (the source of truth);
//! - owner fields are reused from the cached `LockEnv` when present, its
//!   `universe_id` still matches today's value, AND it actually holds an owner
//!   when one is being asked for;
//! - on a miss, we either fetch from Roblox (`fetch_owners = true`, used by
//!   keys whose scopes need creator-target resolution) or fall back to a
//!   placeholder `(User, 0)` that mirrors v3's "no owner needed" behavior.
//!
//! That third condition is the fix for a latent bug rather than an obvious
//! rule. The placeholder exists because the schema cannot say "not asked", and
//! for a long time the cache test was `universe_id` alone, so the first key
//! on an env with no creator-target scope wrote `(User, 0)`, and every later
//! key that *did* need the owner found a matching `universe_id` and reused it.
//! The resulting scope targets `U0`. Treating the placeholder as a miss also
//! repairs lockfiles already carrying one, on the first command that needs a
//! real owner, with no migration.
//!
//! The returned map only covers `env_names`. Callers merge it into the live
//! `lock.envs` so envs referenced by other keys persist untouched.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use rbx_core::owner::OwnerType;
use rbx_core::places::PlacesFile;

use crate::api::RbxApiKeyClient;
use crate::lock::LockEnv;

/// The cached entry to reuse, if it is usable for what this caller needs.
///
/// Separate from `sync_envs` because it is the whole of the decision and none
/// of the I/O: the miss it returns leads to a Roblox call, so the conditions
/// are worth testing without one.
fn reusable_cache_entry(
    cached: Option<&LockEnv>,
    universe_id: u64,
    fetch_owners: bool,
) -> Option<&LockEnv> {
    let cached = cached?;
    if cached.universe_id != universe_id {
        // The env has been repointed at another universe; its owner is a
        // different creator's.
        return None;
    }
    if fetch_owners && cached.owner_is_placeholder() {
        // Written by an earlier caller that needed no owner. Reusing it here
        // is how a scope ends up targeting `U0`.
        return None;
    }
    Some(cached)
}

pub async fn sync_envs(
    client: &RbxApiKeyClient,
    env_names: &[String],
    places: &PlacesFile,
    cached: &BTreeMap<String, LockEnv>,
    fetch_owners: bool,
) -> Result<BTreeMap<String, LockEnv>> {
    let mut out: BTreeMap<String, LockEnv> = BTreeMap::new();
    for env_name in env_names {
        let env = places.get(env_name).with_context(|| {
            format!("env '{env_name}' (referenced by key) not found in rbxplace.toml")
        })?;
        let universe_id = env.universe_id;

        if let Some(reusable) =
            reusable_cache_entry(cached.get(env_name), universe_id, fetch_owners)
        {
            out.insert(env_name.clone(), reusable.clone());
            continue;
        }

        let lock_env = if fetch_owners {
            let owner = client.fetch_universe_owner(universe_id).await?;
            LockEnv {
                universe_id,
                owner_type: owner.owner_type,
                owner_id: owner.owner_id,
            }
        } else {
            LockEnv {
                universe_id,
                owner_type: OwnerType::User,
                owner_id: 0,
            }
        };
        out.insert(env_name.clone(), lock_env);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(universe_id: u64) -> LockEnv {
        LockEnv {
            universe_id,
            owner_type: OwnerType::Group,
            owner_id: 42,
        }
    }

    fn placeholder(universe_id: u64) -> LockEnv {
        LockEnv {
            universe_id,
            owner_type: OwnerType::User,
            owner_id: 0,
        }
    }

    #[test]
    fn a_resolved_entry_is_reused_whether_or_not_the_caller_needs_the_owner() {
        let cached = resolved(100);
        assert!(reusable_cache_entry(Some(&cached), 100, true).is_some());
        assert!(reusable_cache_entry(Some(&cached), 100, false).is_some());
    }

    /// The caller does not need an owner, so the placeholder is a correct
    /// answer and costs no request.
    #[test]
    fn a_placeholder_is_reused_when_no_owner_is_needed() {
        let cached = placeholder(100);
        assert!(reusable_cache_entry(Some(&cached), 100, false).is_some());
    }

    /// The bug this function exists for. Reusing it builds a creator-targeted
    /// scope over `U0`.
    #[test]
    fn a_placeholder_is_a_miss_when_the_owner_is_needed() {
        let cached = placeholder(100);
        assert!(reusable_cache_entry(Some(&cached), 100, true).is_none());
    }

    #[test]
    fn a_repointed_env_is_a_miss_even_with_a_resolved_owner() {
        let cached = resolved(100);
        assert!(reusable_cache_entry(Some(&cached), 999, true).is_none());
    }

    #[test]
    fn nothing_cached_is_a_miss() {
        assert!(reusable_cache_entry(None, 100, false).is_none());
    }
}
