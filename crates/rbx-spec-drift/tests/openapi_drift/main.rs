//! Early-warning alarm for Roblox API drift.
//!
//! This test extracts every Roblox endpoint URL this workspace actually calls
//! and asserts each one still exists in the vendored OpenAPI document at
//! `spec/openapi.json`. When Roblox moves or removes an endpoint, this test
//! goes red in CI instead of a user hitting `Failed to parse response` at
//! runtime.
//!
//! It reads source files as plain text and the spec as plain JSON. It compiles
//! against nothing in this workspace on purpose, so it keeps working while
//! other crates are mid-refactor.
//!
//! # What this checks, honestly
//!
//! The vendored document is broader than its `cloud/v2` reputation suggests.
//! Its top-level `servers` entry is `https://apis.roblox.com`, but individual
//! operations override that with per-operation `servers` blocks naming the
//! legacy hosts: `develop.roblox.com`, `games.roblox.com`,
//! `groups.roblox.com`, `badges.roblox.com`, `thumbnails.roblox.com`,
//! `users.roblox.com`, `assetdelivery.roblox.com`, `economy.roblox.com` and
//! others. So a large part of our legacy surface *is* checkable here, and this
//! test checks it. Matching is host-aware: a path is only considered present
//! if the spec documents it **on the same host** we call it on. Without that,
//! our `games.roblox.com/v1/games` would spuriously "match" an unrelated
//! `/v1/games` documented on a different host.
//!
//! # The URL shapes it recognises
//!
//! Extraction is a text scan, so it only sees the shapes it was taught. These
//! are all of them, and a call written any other way contributes **nothing**
//! to this test while still compiling and running fine:
//!
//! | shape | example |
//! | --- | --- |
//! | an absolute URL in one literal | `"https://games.roblox.com/v1/games"` |
//! | a `const` base interpolated by name | `format!("{BASE}/v1/games/{id}")` |
//! | a `const` base interpolated positionally | `format!("{}/v1/games", BASE)` |
//! | a relative path within three lines of a `.join(` | `base.join(&format!("/cloud/v2/universes/{id}"))` |
//!
//! Two supporting conventions make those work: a `const` holding an absolute
//! URL is a *prefix*, never an endpoint on its own, and a non-default
//! `ApiBase` field named `<name>` is fed by a `const <NAME>_HOST` so that
//! `self.develop.join(...)` resolves to the right host rather than to Open
//! Cloud (see `join_receiver_host`).
//!
//! **Adding a new shape is a change to this file too.** A helper that
//! concatenates path fragments, a table of `const` endpoints, a builder: none
//! of them are recognised, and the crate that introduces one silently drops
//! out of the checked set. Two guards exist for that: `MINIMUM_ENDPOINTS`
//! catches gross breakage, and
//! [`every_crate_that_calls_roblox_contributes_an_endpoint`] catches the
//! narrow case of one crate, which is how the net actually erodes.
//!
//! # What this deliberately does not check
//!
//! - **`crates/rbx-probe/`** is skipped. It exists to send a raw request to an
//!   arbitrary user-supplied path; its paths are inputs, not call sites, so
//!   there is nothing to verify against the spec.
//! - **`crates/*/tests/`** is skipped. Those paths point at wiremock servers.
//! - **`#[cfg(test)]` items** are skipped, one item at a time: the attribute
//!   and the item it guards are blanked out, and the rest of the file is still
//!   read. Unit tests assert against fixed ids (`/cloud/v2/universes/1`),
//!   which are not real endpoint templates. This used to cut the file at the
//!   first marker and keep nothing after it, which cost every client that
//!   gated a `with_base_url` helper near the top of its module the whole of
//!   its coverage: `rbx-place`'s upload, rollback and download went with one
//!   such helper (`crates/rbx-place/src/api/mod.rs:70`, no longer
//!   `#[cfg(test)]`), and so did the configs client's publish and revisions
//!   calls, which now live in `crates/rbx-core/src/api/configs.rs`.
//! - **`www.roblox.com` and `create.roblox.com`** are skipped. Every use of
//!   those is a human-facing web link (a profile URL, a dashboard link) or an
//!   `Origin`/`Referer` header, not an API call.
//! - **Endpoints assembled from a `const` base plus runtime pieces** are only
//!   resolved for the simple, single-line `format!` shapes this codebase
//!   actually uses (`format!("{BASE}/x")` and `format!("{}/x", BASE)`). A
//!   `const` declaration on its own is treated as a prefix, never as an
//!   endpoint, because most of them are (`".../universes/v1"`). This is the
//!   main blind spot; see `MINIMUM_ENDPOINTS` for the guard against it
//!   silently widening.
//! - **Undocumented-by-Roblox endpoints** are listed in `KNOWN_UNDOCUMENTED`
//!   with a reason each. They are real endpoints we call that this document
//!   has never described, so they cannot be verified against it. They are not
//!   evidence of drift, and an alarm that is red on day one gets ignored.
//!
//! # Refreshing the spec
//!
//! Do not hand-edit `spec/openapi.json`. The `update-openapi` workflow
//! re-fetches it and updates `spec/source.json`. See `spec/NOTICE.md`.

mod bodies;
mod configs;
mod coverage;
mod extract;
mod extractor_guards;
mod paths;
mod spec;

use crate::{extract::*, paths::*, spec::*};
use std::collections::BTreeSet;

/// HTTP methods that mark a key inside an OpenAPI path item as an operation.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// Hosts we build URLs for that are never API calls.
const NON_API_HOSTS: &[&str] = &["https://www.roblox.com", "https://create.roblox.com"];

/// Crate directories excluded from extraction, with the reason.
const SKIPPED_CRATES: &[(&str, &str)] = &[
    (
        "rbx-probe",
        "builds arbitrary user-supplied paths by design; its paths are inputs, not call sites",
    ),
    (
        "rbx-spec-drift",
        "this crate; the strings below are examples",
    ),
];

/// Endpoints we call that this document has never documented. Each entry is
/// `(host, path, reason)`. Paths are matched structurally, so `{}` here means
/// "any templated segment", exactly as in the extraction.
///
/// Adding to this list is a deliberate act: it says "Roblox does not describe
/// this endpoint, so we accept that we cannot get an early warning for it".
/// Removing an entry is free, if Roblox documents one of these later, the
/// test simply starts covering it, and a stale entry costs nothing but a line.
const KNOWN_UNDOCUMENTED: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/api-keys/v1/introspect",
        "API key introspection; used by `rbx apikey`, never published in the OpenAPI document",
    ),
    (
        "https://apis.roblox.com",
        "/cloud-authentication/v1/apiKey",
        "API key create/update/delete, used by `rbx apikey`; the cookie-authenticated \
         management API behind the Creator Hub credentials page, never published in the \
         OpenAPI document (same family as /api-keys/v1/introspect above)",
    ),
    (
        "https://apis.roblox.com",
        "/cloud-authentication/v1/apiKeys",
        "plural, reached with a POST: the key listing behind the same page, undocumented \
         for the same reason",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/places/{}/universe",
        "legacy place -> universe resolution; the document only describes \
         /universes/v1/{}/places/{}/versions",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/universes/create",
        "legacy universe creation, used by `rbx init`; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/universes/v1/user/universes/{}/places",
        "legacy place creation, used by `rbx init`; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api/release_status",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/experience-releases/v1beta1/experience_releases_api/release_status/{}",
        "v1beta1 release-status API behind the creator dashboard; undocumented",
    ),
    (
        "https://apis.roblox.com",
        "/developer-products/v2/universes/{}/developerproducts",
        "unhyphenated legacy spelling used by the public (unauthenticated) listing in \
         rbx-shop/src/api/public.rs; the document only describes the hyphenated \
         /developer-products/v2/universes/{}/developer-products, which rbx-shop/src/api/products.rs \
         already uses. Worth collapsing onto the documented spelling.",
    ),
    (
        "https://apis.roblox.com",
        "/analytics-query-api/{}",
        "not a fixed endpoint: this is the long-running-operation poll in rbx-analytics, whose \
         tail is an operation path handed back by Roblox at runtime. There is no static path \
         to verify. The analytics endpoints that *are* static \
         (/analytics-query-api/v1/universes/{}/metrics and .../dimension-values) are checked.",
    ),
    (
        "https://economy.roblox.com",
        "/v2/assets/{}/details",
        "economy.roblox.com is a documented host, but this particular path is not among the \
         operations it documents (only /v2/assets/batch, /v2/assets/{}/owners, \
         /v2/assets/{}/versions)",
    ),
];

/// A floor on how many endpoints extraction must find.
///
/// Without this, a refactor that changes how URLs are written would quietly
/// reduce the test to checking nothing, and it would still pass. If this trips
/// after a legitimate change, read the extraction rules above before lowering
/// it: the usual cause is a new URL-building shape that needs supporting, not
/// a number that needs editing.
const MINIMUM_ENDPOINTS: usize = 60;

/// Crates that depend on the HTTP layer and still contribute no endpoint to
/// this check, with the reason each is legitimate.
///
/// The floor above only catches gross breakage. The failure mode in between is
/// one crate: a new domain crate builds its URLs with a shape the extractor
/// does not recognise, contributes zero, and nothing goes red: the net
/// narrows by one crate while the total stays comfortably above the floor.
/// [`every_crate_that_calls_roblox_contributes_an_endpoint`] closes that, and
/// this list is where a genuine exception is admitted out loud.
///
/// An entry here is a claim that the crate calls no Roblox endpoint of its
/// own, not that its endpoints are hard to extract. If the extractor cannot
/// see a call the crate really makes, teach the extractor.
///
/// **Empty on purpose.** It held `rbx-config` for as long as that crate owned
/// the configs client, and the entry outlived the fact: the client moved to
/// `rbx_core::api::configs`, `rbx-config` stopped touching HTTP at all, and
/// what was left was an exemption covering a crate that could not call Roblox
/// while the configs surface itself went unchecked inside a crate that passed.
/// The endpoints are covered directly now, by
/// [`every_configs_endpoint_is_documented_in_the_spec`], which is what an
/// exemption should turn into.
const CRATES_WITHOUT_ENDPOINTS: &[(&str, &str)] = &[];

const CALLED_THROUGH_A_HELPER: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/scopes/{scope_id}/entries/{entry_id}:increment",
        "rbx-data/src/lib.rs, `format!(\"{}:increment\", self.entry_url(entry))`. \
         Reached by `rbx data increment`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/data-stores/{data_store_id}/scopes/{scope_id}/entries/{entry_id}:listRevisions",
        "rbx-data/src/lib.rs, `format!(\"{}:listRevisions?maxPageSize=100\", \
         self.entry_url(entry))`. Reached by `rbx data revisions`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universeId}/secrets/{secretId}",
        "rbx-secret/src/lib.rs, `format!(\"{}/{}\", self.secrets_url(), \
         encode_query_value(id))`. Reached by `rbx secret set` (the PATCH half \
         of its POST-then-PATCH upsert) and `rbx secret remove`.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}/memory-store/sorted-maps/{sorted_map_id}/items/{item_id}",
        "rbx-memorystore/src/lib.rs, `fn item_url` appending an id to \
         `items_url()`. Reached by the get, update and delete of `rbx \
         memorystore`.",
    ),
];

/// Endpoints inside a family this workspace covers that it deliberately does
/// not call, each with the reason.
///
/// This is the counterpart of [`KNOWN_UNDOCUMENTED`] and works the same way: an
/// entry is a decision somebody made and can be argued with, where a silent
/// omission is a decision nobody recorded.
///
/// It stays short by construction. The scope is not curated here, it is read
/// off the code: an endpoint is only ever reported when the workspace already
/// calls something in the same family, so the hundreds of documented endpoints
/// in areas this tool has no business in (trading, private messages, avatar
/// customisation, localization tables) contribute nothing and need no entry.
const NOT_CALLED_ON_PURPOSE: &[(&str, &str, &str)] = &[
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:restartServers",
        "A second way to do what `rbx restart` already does through \
         `/server-management/v1/universes/{id}/restarts`. The one in use also \
         forecasts and reports status, which this does not, so switching would \
         trade a three-endpoint command for a one-endpoint one. Worth \
         revisiting only if Roblox deprecates the server-management surface.",
    ),
    (
        "https://apis.roblox.com",
        "/assets/v1/assets/{assetId}/versions/{versionNumber}",
        "Metadata for one asset version. `rbx place` lists versions through the \
         sibling path and fetches the bytes of a chosen one through \
         `asset-delivery-api`, so the middle step has nothing to add: nothing \
         here asks a question that the listing has not already answered.",
    ),
    (
        "https://apis.roblox.com",
        "/ads-management/v1/billing-accounts/{id}",
        "One billing account by id. `rbx ads` lists them to let a campaign name \
         one, and never needs to look one up afterwards.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:generateSpeechAsset",
        "Text to speech. Content creation, not deployment or operations, and \
         nothing else in this tool generates assets.",
    ),
    (
        "https://apis.roblox.com",
        "/cloud/v2/universes/{universe_id}:translateText",
        "Machine translation of a string. Same reason: this tool moves and \
         configures what a project already has.",
    ),
];

fn declined(host: &str, path: &str) -> Option<&'static str> {
    let segments = normalise(path);
    NOT_CALLED_ON_PURPOSE
        .iter()
        .chain(CALLED_THROUGH_A_HELPER)
        .find(|(h, p, _)| *h == host && normalise(p) == segments)
        .map(|(_, _, reason)| *reason)
}
// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

/// An endpoint we call that the spec does not describe: `(host, path, call sites)`.
type Missing<'a> = (String, String, &'a BTreeSet<(String, usize)>);

#[test]
fn every_endpoint_we_call_still_exists_in_the_roblox_spec() {
    let root = repo_root();
    let (index, provenance) = load_spec(&root);
    let sites = collect_call_sites(&root);

    let mut checked = 0usize;
    let mut skipped = 0usize;
    let mut missing: Vec<Missing<'_>> = Vec::new();

    for ((host, path), locations) in &sites {
        if is_known_undocumented(host, path).is_some() {
            skipped += 1;
            continue;
        }
        checked += 1;
        let key = (host.clone(), normalise(path));
        if !index.contains_key(&key) {
            missing.push((host.clone(), path.clone(), locations));
        }
    }

    assert!(
        checked + skipped >= MINIMUM_ENDPOINTS,
        "The drift check only extracted {} endpoint(s) from crates/, which is below the \
         floor of {}.\n\n\
         This almost certainly means extraction is broken, not that the codebase shrank: \
         a new way of building URLs was introduced that the extractor in {} does not \
         recognise, so it is now silently checking almost nothing.\n\
         Read the module docs in that file (\"What this deliberately does not check\") and \
         teach it the new shape. Do not simply lower the floor.",
        checked + skipped,
        MINIMUM_ENDPOINTS,
        file!(),
    );

    if !missing.is_empty() {
        let mut report = String::new();
        report.push_str(&format!(
            "\n\nRoblox Open Cloud API drift detected.\n\
             \n{} endpoint(s) this workspace calls are NOT present in the vendored OpenAPI \
             document.\nVendored spec: {}\n\n",
            missing.len(),
            provenance,
        ));

        for (host, path, locations) in &missing {
            report.push_str(&format!("  {host}{path}\n"));
            for (file, line) in locations.iter() {
                report.push_str(&format!("      called from {file}:{line}\n"));
            }
            // Offer near misses: same path shape on a different host, or the
            // same host with a similar path. This usually names the rename.
            let segments = normalise(path);
            let mut scored: Vec<(usize, usize, String)> = index
                .iter()
                .filter_map(|((h, s), spec_path)| {
                    if *s == segments && h != host {
                        // Same shape, different host: Roblox moved the service.
                        return Some((usize::MAX, usize::MAX, format!("{h}{spec_path}")));
                    }
                    if h != host || s.len() != segments.len() {
                        return None;
                    }
                    let prefix = s.iter().zip(&segments).take_while(|(a, b)| a == b).count();
                    let suffix = s
                        .iter()
                        .rev()
                        .zip(segments.iter().rev())
                        .take_while(|(a, b)| a == b)
                        .count();
                    // Two shared segments is the point where a suggestion
                    // stops being noise.
                    if prefix + suffix < 2 {
                        return None;
                    }
                    // Among equally-shaped candidates, prefer the one whose
                    // differing segments most resemble ours: that is what a
                    // rename looks like.
                    let similarity: usize = s
                        .iter()
                        .zip(&segments)
                        .filter(|(a, b)| a != b)
                        .map(|(a, b)| longest_common_substring(a, b))
                        .sum();
                    Some((prefix + suffix, similarity, format!("{h}{spec_path}")))
                })
                .collect();
            scored.sort_by(|a, b| {
                b.0.cmp(&a.0)
                    .then_with(|| b.1.cmp(&a.1))
                    .then_with(|| a.2.cmp(&b.2))
            });
            let mut hints: Vec<String> = scored.into_iter().map(|(_, _, h)| h).collect();
            hints.dedup();
            hints.truncate(5);
            if !hints.is_empty() {
                report.push_str("      did Roblox mean one of these?\n");
                for hint in hints {
                    report.push_str(&format!("        {hint}\n"));
                }
            }
            report.push('\n');
        }

        report.push_str(
            "What to do:\n\
             \n  1. Check whether Roblox MOVED this endpoint (a rename or a new version) or\n\
             \x20    REMOVED it. The upstream changelog and the paths listed above are the\n\
             \x20    fastest way to tell.\n\
             \n  2. If it moved, update the call site(s) named above to the new path, and the\n\
             \x20    response types with it: a moved endpoint often reshapes its payload, which\n\
             \x20    is what produces `Failed to parse response` for users.\n\
             \n  3. If it was removed with no replacement, the feature that depends on it is\n\
             \x20    broken for every user. Decide whether to drop it or move to a different API.\n\
             \n  4. If Roblox merely stopped DOCUMENTING an endpoint that still works, add it to\n\
             \x20    KNOWN_UNDOCUMENTED in this file with a one-line reason. Do that only after\n\
             \x20    confirming it still works: that list is how we admit we have no early\n\
             \x20    warning for something, not a way to silence this test.\n\
             \n  5. If the spec was just refreshed and you did not expect any of this, check\n\
             \x20    `git log -1 -- spec/openapi.json` and diff the two revisions of the paths\n\
             \x20    above before changing any code.\n",
        );

        panic!("{report}");
    }

    println!(
        "Checked {checked} endpoint(s) against {provenance}; \
         {skipped} known-undocumented endpoint(s) skipped."
    );
}

/// Every crate that can call Roblox must contribute at least one endpoint.
///
/// The floor in the test above is a workspace-wide total, which one crate
/// cannot move: a domain crate whose URLs the extractor does not recognise
/// drops out silently while the total stays comfortably above 35. This is the
/// per-crate version of the same guard, and it fails on the day a crate stops
/// being covered rather than months later during an audit.
///
/// Two ways out when it goes red, and only one of them is usually right:
/// teach the extractor the new shape (see the module docs), or admit in
/// `CRATES_WITHOUT_ENDPOINTS` that the crate genuinely calls nothing.
#[test]
fn every_crate_that_calls_roblox_contributes_an_endpoint() {
    let root = repo_root();
    let sites = collect_call_sites(&root);
    let counts = endpoints_per_crate(&sites);
    let http = http_crates(&root);

    assert!(
        !http.is_empty(),
        "no crate under {}/crates depends on reqwest, which cannot be true: the manifest scan \
         in http_crates() is broken, so this test is checking nothing.",
        root.display()
    );

    let exempt = |krate: &str| {
        SKIPPED_CRATES.iter().any(|(name, _)| *name == krate)
            || CRATES_WITHOUT_ENDPOINTS
                .iter()
                .any(|(name, _)| *name == krate)
    };

    let silent: Vec<&String> = http
        .iter()
        .filter(|krate| !exempt(krate))
        .filter(|krate| counts.get(krate.as_str()).copied().unwrap_or(0) == 0)
        .collect();

    assert!(
        silent.is_empty(),
        "these crates depend on the HTTP layer but contributed no endpoint to the drift \
         check, so nothing they call is verified against the Roblox spec:\n\n{}\n\n\
         The usual cause is a URL-building shape the extractor in {} does not recognise: \
         a helper that concatenates paths, a const table, a different formatting macro. \
         Read the module docs (\"The URL shapes it recognises\") and teach it the shape.\n\
         If the crate really calls no Roblox endpoint of its own, add it to \
         CRATES_WITHOUT_ENDPOINTS with the reason.",
        silent
            .iter()
            .map(|krate| format!("  crates/{krate}"))
            .collect::<Vec<_>>()
            .join("\n"),
        file!(),
    );

    // The other direction: an exemption that stopped being true is a lie in a
    // file whose whole job is to be trusted.
    let stale: Vec<&str> = CRATES_WITHOUT_ENDPOINTS
        .iter()
        .map(|(krate, _)| *krate)
        .filter(|krate| counts.get(*krate).copied().unwrap_or(0) > 0)
        .collect();

    assert!(
        stale.is_empty(),
        "these crates are listed in CRATES_WITHOUT_ENDPOINTS but do contribute endpoints \
         now: {}. Remove them from the list.",
        stale.join(", ")
    );

    let unknown: Vec<&str> = CRATES_WITHOUT_ENDPOINTS
        .iter()
        .map(|(krate, _)| *krate)
        .filter(|krate| !http.contains(*krate))
        .collect();

    assert!(
        unknown.is_empty(),
        "these crates are listed in CRATES_WITHOUT_ENDPOINTS but no longer depend on the HTTP \
         layer (renamed, deleted, or the dependency was dropped): {}. Remove them from the \
         list: an exemption for a crate that cannot call Roblox anyway hides nothing and \
         outlives the reason it was written.",
        unknown.join(", ")
    );

    println!(
        "{} crate(s) with an HTTP dependency, {} contributing endpoints, {} exempt.",
        http.len(),
        http.iter()
            .filter(|k| counts.contains_key(k.as_str()))
            .count(),
        http.iter().filter(|k| exempt(k)).count(),
    );
}

/// The reason one of `KNOWN_UNDOCUMENTED`'s entries covers this call, if one
/// does.
///
/// Lives beside that list rather than with the extractor: it is a question
/// about a decision somebody wrote down, not about reading source.
fn is_known_undocumented(host: &str, path: &str) -> Option<&'static str> {
    let segments = normalise(path);
    KNOWN_UNDOCUMENTED
        .iter()
        .find(|(h, p, _)| *h == host && normalise(p) == segments)
        .map(|(_, _, reason)| *reason)
}
