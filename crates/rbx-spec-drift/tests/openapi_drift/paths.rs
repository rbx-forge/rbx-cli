//! Turning a path into something two sides can be compared on.
//!
//! Our Rust writes a parameter where the spec writes another spelling of it, so
//! a segment carrying one is collapsed to a wildcard and only the literal
//! segments have to agree.

/// Collapses every `{...}` run in a segment to `*`, so that our Rust variable
/// names and the spec's parameter names never have to agree.
///
/// `{universe_id}` -> `*`, `{}` -> `*`, `{entry_id}:increment` -> `*:increment`,
/// `data-stores:snapshot` -> `data-stores:snapshot`.
pub(crate) fn normalise_segment(segment: &str) -> String {
    let mut out = String::new();
    let mut chars = segment.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            for inner in chars.by_ref() {
                if inner == '}' {
                    break;
                }
            }
            out.push('*');
        } else {
            out.push(c);
        }
    }
    out
}

/// Length of the longest common substring. Used only to rank suggestions, so
/// that a one-segment rename (`user-restrictions` -> `player-restrictions`)
/// outranks an unrelated path of the same shape (`places`).
pub(crate) fn longest_common_substring(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut previous = vec![0usize; b.len() + 1];
    let mut best = 0;
    for i in 1..=a.len() {
        let mut current = vec![0usize; b.len() + 1];
        for j in 1..=b.len() {
            if a[i - 1] == b[j - 1] {
                current[j] = previous[j - 1] + 1;
                best = best.max(current[j]);
            }
        }
        previous = current;
    }
    best
}

/// Splits a path into normalised segments: literal segments must match
/// exactly, templated segments become wildcards.
pub(crate) fn normalise(path: &str) -> Vec<String> {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.trim_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(normalise_segment)
        .collect()
}
/// dropped.
///
/// `/cloud/v2/universes/{universeId}/data-stores` and
/// `/cloud/v2/universes/{universeId}/data-stores/{dataStoreId}` share a family,
/// because the second is an operation on the resource the first lists. A
/// sub-resource does not: `.../data-stores/{dataStoreId}/entries` keeps
/// `entries` and is its own family, so calling the entries API says nothing
/// about whether the store API is covered.
///
/// A segment like `{dataStoreId}:undelete` normalises to `*:undelete`, which
/// carries a wildcard and is therefore dropped too. That is deliberate: the
/// `:undelete` action belongs to the resource it acts on, and it is one of the
/// endpoints this check was asked to surface.
pub(crate) fn family(segments: &[String]) -> Vec<String> {
    segments
        .iter()
        .filter(|segment| !segment.contains('*'))
        .cloned()
        .collect()
}
