//! What `import` could not bring across, said out loud.
//!
//! The house rule is that "an ignored key must not pass for applied"
//! (`docs/env.md`). A partial import is the same defect one level up: a
//! directory that looks adopted but silently omits the thing you most needed
//! is worse than one that failed, because nothing prompts you to look.
//!
//! So every domain that is skipped, and every field Open Cloud has no coverage
//! for, is named at the end of the run with the reason and, where there is
//! one: what to do about it.

use colored::Colorize;

use crate::Domain;

/// One thing that did not make it into the imported files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap {
    /// Where it belonged.
    pub domain: Domain,
    /// What is missing, in the user's vocabulary: a file, a field, a group of
    /// resources.
    pub subject: String,
    /// Why. Written to be read by somebody who did not write this code.
    pub reason: String,
    /// What would fix it, when anything would.
    pub remedy: Option<String>,
}

impl Gap {
    pub fn new(domain: Domain, subject: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            domain,
            subject: subject.into(),
            reason: reason.into(),
            remedy: None,
        }
    }

    pub fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remedy = Some(remedy.into());
        self
    }
}

/// The fields no API this tool can reach will return without a browser
/// session, listed once so the report says the same thing every run.
///
/// These are not failures (`meta` models them and `sync` can write them) but
/// an import that leaves them blank has to say so, or the first `meta sync`
/// after an import looks like it is inventing changes.
pub fn cookie_only_meta_gaps(has_cookie: bool) -> Vec<Gap> {
    if has_cookie {
        return Vec::new();
    }
    vec![Gap::new(
        Domain::Meta,
        "server fill, copying permission, beta mode",
        "these live only on legacy endpoints that need a Roblox session cookie",
    )
    .with_remedy("re-run with --cookie, or set them by hand in rbxmeta.toml")]
}

/// Print the report. Nothing is printed when there is nothing to say: a clean
/// import should not end on a list of headings.
pub fn print(gaps: &[Gap]) {
    if gaps.is_empty() {
        println!("{} Everything reachable was imported.", "✓".green());
        return;
    }

    println!(
        "\n{} {} thing{} could not be imported:",
        "!".yellow(),
        gaps.len(),
        if gaps.len() == 1 { "" } else { "s" }
    );
    for gap in gaps {
        println!(
            "  {} {}: {}",
            gap.domain.label().dimmed(),
            gap.subject.bold(),
            gap.reason
        );
        if let Some(remedy) = &gap.remedy {
            println!("    {} {}", "->".dimmed(), remedy.dimmed());
        }
    }
    println!(
        "\n{}",
        "Everything else is under management; `check` covers it from here.".dimmed()
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_cookie_removes_the_legacy_field_gap() {
        assert!(cookie_only_meta_gaps(true).is_empty());
        let without = cookie_only_meta_gaps(false);
        assert_eq!(without.len(), 1);
        assert_eq!(without[0].domain, Domain::Meta);
        assert!(without[0].remedy.is_some(), "a gap with a fix must name it");
    }

    /// The report is the whole point of the requirement, so its shape is
    /// asserted rather than left to a visual check.
    #[test]
    fn a_gap_carries_domain_subject_and_reason() {
        let gap = Gap::new(Domain::Shop, "12 badges", "no Open Cloud read endpoint")
            .with_remedy("import them by hand");
        assert_eq!(gap.domain, Domain::Shop);
        assert_eq!(gap.subject, "12 badges");
        assert_eq!(gap.reason, "no Open Cloud read endpoint");
        assert_eq!(gap.remedy.as_deref(), Some("import them by hand"));
    }
}
