//! The other direction: what the spec documents in an area we already work in
//! and never call.

use std::collections::BTreeSet;

use crate::{declined, CALLED_THROUGH_A_HELPER, NOT_CALLED_ON_PURPOSE};
use crate::{extract::*, paths::*, spec::*};

/// What the vendored spec documents, in an area this workspace already works
/// in, and never calls.
///
/// The sibling check answers "does everything we call still exist". It cannot
/// answer this one, because an endpoint nobody calls contributes nothing to a
/// scan of call sites, so the spec can document a capability for months and
/// nothing says so. `Cloud_ListDataStores` sat there unimplemented until
/// somebody asked in conversation how to list an experience's data stores; it
/// shipped as `rbx data stores` in 0.6.0, and nothing in CI had ever mentioned
/// it.
///
/// **Scope is derived, not curated.** An endpoint is reported only when this
/// workspace already calls something in the same [`family`], so the several
/// hundred documented endpoints in areas this tool has no business in stay
/// silent without needing a line each. What that buys is the useful half: a
/// newly documented endpoint next to one already in use shows up once, as
/// "here is something you could now call", instead of never.
#[test]
fn every_documented_endpoint_in_a_covered_area_is_called_or_declined() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);
    let sites = collect_call_sites(&root);

    // The families this workspace works in, and the exact endpoints it calls.
    let mut covered: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    let mut called: BTreeSet<(String, Vec<String>)> = BTreeSet::new();
    for (host, path) in sites.keys() {
        let segments = normalise(path);
        covered.insert((host.clone(), family(&segments)));
        called.insert((host.clone(), segments));
    }

    let mut uncalled: Vec<(String, String)> = Vec::new();
    let mut listed = 0usize;

    for ((host, segments), spec_path) in &index {
        if !covered.contains(&(host.clone(), family(segments))) {
            continue;
        }
        if called.contains(&(host.clone(), segments.clone())) {
            continue;
        }
        if declined(host, spec_path).is_some() {
            listed += 1;
            continue;
        }
        uncalled.push((host.clone(), spec_path.clone()));
    }

    assert!(
        uncalled.is_empty(),
        "\n\n{} documented endpoint(s) sit in an area this workspace already \
         works in and are never called.\nVendored spec: {}\n\n{}\n\n\
         Each one is either a capability worth adding, or a deliberate pass \
         that belongs in NOT_CALLED_ON_PURPOSE with the reason written out. If \
         it is called through a URL built from a helper, it belongs in \
         CALLED_THROUGH_A_HELPER instead, which is a different statement. {} \
         endpoint(s) are already listed between the two.\n\n\
         This list only grows when Roblox documents something new next to \
         something already in use, which is exactly when it is worth reading.",
        uncalled.len(),
        provenance,
        uncalled
            .iter()
            .map(|(host, path)| format!("  {host}{path}"))
            .collect::<Vec<_>>()
            .join("\n"),
        listed,
    );
}

/// An entry that stops describing anything has to come out, the rule every
/// allowlist in this workspace follows. An endpoint listed as deliberately
/// skipped and then implemented is the good outcome; leaving the row behind
/// would exempt it again the day the call is deleted.
///
/// [`CALLED_THROUGH_A_HELPER`] is deliberately not checked this way. Those
/// endpoints *are* called, and the extractor not seeing them is the entire
/// point of the list, so the same assertion would fail every one of them.
/// Their stale-entry check is the opposite one, below.
#[test]
fn nothing_is_declined_while_being_called() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let called: BTreeSet<(String, Vec<String>)> = sites
        .keys()
        .map(|(host, path)| (host.clone(), normalise(path)))
        .collect();

    let stale: Vec<String> = NOT_CALLED_ON_PURPOSE
        .iter()
        .filter(|(host, path, _)| called.contains(&((*host).to_string(), normalise(path))))
        .map(|(host, path, why)| format!("  {host}{path}\n    it was listed as: {why}"))
        .collect();

    assert!(
        stale.is_empty(),
        "{} NOT_CALLED_ON_PURPOSE entry/entries describe nothing:\n{}",
        stale.len(),
        stale.join("\n")
    );
}

/// A helper-built URL that became literal has to leave the list.
///
/// That is the good outcome for [`CALLED_THROUGH_A_HELPER`]: the extractor can
/// see the call, so the sibling drift check now protects it, and the entry is
/// describing a hole that no longer exists. Left behind, it would quietly
/// exempt the endpoint from this check for a reason that stopped being true.
#[test]
fn nothing_is_listed_as_helper_built_once_the_extractor_can_see_it() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let called: BTreeSet<(String, Vec<String>)> = sites
        .keys()
        .map(|(host, path)| (host.clone(), normalise(path)))
        .collect();

    let visible: Vec<String> = CALLED_THROUGH_A_HELPER
        .iter()
        .filter(|(host, path, _)| called.contains(&((*host).to_string(), normalise(path))))
        .map(|(host, path, where_from)| {
            format!("  {host}{path}\n    was listed as unresolvable: {where_from}")
        })
        .collect();

    assert!(
        visible.is_empty(),
        "{} CALLED_THROUGH_A_HELPER entry/entries are extractable now, so the \
         sibling drift check covers them and the row is stale:\n{}",
        visible.len(),
        visible.join("\n")
    );
}

/// Both lists name paths the spec actually documents.
///
/// Without this, a Roblox rename turns an entry into a row matching nothing:
/// the endpoint quietly leaves the report, the reason stays in the file, and
/// the two never meet again. The sibling check catches a rename under a call
/// site; nothing would catch one under an allowlist row.
#[test]
fn every_listed_endpoint_still_exists_in_the_spec() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);

    let unknown: Vec<String> = NOT_CALLED_ON_PURPOSE
        .iter()
        .chain(CALLED_THROUGH_A_HELPER)
        .filter(|(host, path, _)| !index.contains_key(&((*host).to_string(), normalise(path))))
        .map(|(host, path, _)| format!("  {host}{path}"))
        .collect();

    assert!(
        unknown.is_empty(),
        "{} listed endpoint(s) are in neither list's spec any more.\n\
         Vendored spec: {}\n{}\n\n\
         Roblox renamed or removed them. Update the row to the new path, or \
         delete it if the endpoint is gone.",
        unknown.len(),
        provenance,
        unknown.join("\n")
    );
}
