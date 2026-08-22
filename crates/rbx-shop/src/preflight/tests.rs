//! The matching rule and the message it produces.
//!
//! The end-to-end behaviour (that `sync` lists before it creates, and stops)
//! is asserted in `commands/sync/tests.rs`, where the mock server and the
//! config fixture already live. What is left here is the part that is a
//! judgement call rather than a wiring question: which names count as the
//! same, and whether the refusal tells the reader enough to fix it.

use super::*;

fn creates(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, n)| (k.to_string(), n.to_string()))
        .collect()
}

fn remote(pairs: &[(&str, u64)]) -> Vec<(String, u64)> {
    pairs.iter().map(|(n, id)| (n.to_string(), *id)).collect()
}

#[test]
fn an_empty_experience_collides_with_nothing() {
    let found = collide(
        ResourceKind::Pass,
        &creates(&[("VIP", "VIP"), ("Gold", "Gold")]),
        &remote(&[]),
    );
    assert!(found.is_empty());
}

/// The case this exists for: the lockfile is gone, so every resource looks
/// new, and every one of them is already live.
#[test]
fn a_lost_lockfile_collides_on_every_resource() {
    let found = collide(
        ResourceKind::Pass,
        &creates(&[("VIP", "VIP"), ("Gold", "Gold")]),
        &remote(&[("VIP", 111), ("Gold", 222)]),
    );
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].remote_ids, vec![111]);
    assert_eq!(found[1].remote_ids, vec![222]);
}

/// The key and the display name are different strings, and it is the display
/// name Roblox stores. A resource whose `name` was overridden must still be
/// matched, or the guard misses exactly the resources somebody bothered to
/// name properly.
#[test]
fn matching_is_on_the_display_name_not_the_config_key() {
    let found = collide(
        ResourceKind::Pass,
        &creates(&[("vip_pass", "VIP Pass")]),
        &remote(&[("VIP Pass", 111)]),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].key, "vip_pass");
    assert_eq!(found[0].display_name, "VIP Pass");
}

#[test]
fn a_name_differing_only_in_case_is_the_same_name() {
    let found = collide(
        ResourceKind::Product,
        &creates(&[("coins", "100 COINS")]),
        &remote(&[("100 Coins", 333)]),
    );
    assert_eq!(found.len(), 1);
}

/// A different name is a different resource, and stopping on it would make the
/// guard useless: every first sync of a real shop would need the escape hatch.
#[test]
fn a_different_name_is_not_a_collision() {
    let found = collide(
        ResourceKind::Badge,
        &creates(&[("Welcome", "Welcome")]),
        &remote(&[("Farewell", 999)]),
    );
    assert!(found.is_empty());
}

/// The experience already had duplicates before this run. Reporting only the
/// first id would send the developer to reconcile against one of two.
#[test]
fn every_remote_id_carrying_the_name_is_reported() {
    let found = collide(
        ResourceKind::Pass,
        &creates(&[("VIP", "VIP")]),
        &remote(&[("VIP", 111), ("Other", 222), ("vip", 333)]),
    );
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].remote_ids, vec![111, 333]);
}

/// The message is the whole remedy. If it does not name the ids, the command
/// that repairs the lockfile, and the flag that overrides it, the developer is
/// left with a refusal and no way forward.
#[test]
fn the_refusal_names_the_ids_the_remedy_and_the_override() {
    let found = collide(
        ResourceKind::Pass,
        &creates(&[("VIP", "VIP Pass")]),
        &remote(&[("VIP Pass", 4242)]),
    );
    let message = format!("{:#}", refuse(&found, "prod"));

    assert!(message.contains("4242"), "{message}");
    assert!(message.contains("VIP Pass"), "{message}");
    assert!(message.contains("rbx shop pull --env prod"), "{message}");
    assert!(message.contains("--allow-duplicate-names"), "{message}");
    assert!(
        message.contains(crate::lockfile::LOCKFILE_NAME),
        "{message}"
    );
}
