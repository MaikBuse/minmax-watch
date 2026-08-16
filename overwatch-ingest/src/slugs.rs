//! Per-source URL slug resolution.
//!
//! The sites do not agree with OverFast — or with each other — on how to spell
//! a hero in a URL. counterpickgg calls Freja "freya"; counterwatch drops the
//! hyphen from `jetpack-cat` and `soldier-76` but keeps it in `wrecking-ball`.
//!
//! Rather than hard-coding one mapping per hero per site, each source declares
//! the handful of genuine exceptions and everything else falls back to a short
//! list of mechanical variants that gets tried in order. The site's own spelling
//! never escapes this module: callers work in our canonical keys throughout.

/// counterpickgg's spellings that differ from ours, as `(ours, theirs)`.
const COUNTERPICKGG: &[(&str, &str)] = &[("freja", "freya")];

/// counterwatch's spellings that differ from ours.
const COUNTERWATCH: &[(&str, &str)] = &[("jetpack-cat", "jetpackcat"), ("soldier-76", "soldier76")];

fn variants(key: &str, overrides: &[(&str, &str)]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // Candidates can repeat without being adjacent - an override often *is* the
    // dehyphenated form - so push through a containment check, not `dedup`.
    let mut push = |candidate: String| {
        if !out.contains(&candidate) {
            out.push(candidate);
        }
    };

    if let Some((_, theirs)) = overrides.iter().find(|(ours, _)| *ours == key) {
        push((*theirs).to_owned());
    }
    push(key.to_owned());
    // The common mechanical difference is hyphenation.
    push(key.replace('-', ""));

    out
}

/// URL slugs to try for `key`, best first.
pub fn counterpickgg(key: &str) -> Vec<String> {
    variants(key, COUNTERPICKGG)
}

pub fn counterwatch(key: &str) -> Vec<String> {
    variants(key, COUNTERWATCH)
}

/// Maps a slug seen on counterpickgg back to our canonical hero key.
///
/// Needed because a hero page links to *other* heroes using the site's
/// spelling, so the opponent side of every matchup arrives in their dialect.
pub fn counterpickgg_to_ours(slug: &str) -> String {
    COUNTERPICKGG
        .iter()
        .find(|(_, theirs)| *theirs == slug)
        .map(|(ours, _)| (*ours).to_owned())
        .unwrap_or_else(|| slug.to_owned())
}

/// The same, for counterwatch. Its duo pages link partners by slug rather than
/// by display name, which is the spelling worth reading: it survives `Lúcio`
/// and `Soldier: 76` without a single accent or colon.
///
/// The dehyphenated forms cannot be undone mechanically — `wreckingball` and
/// `jetpackcat` look identical as strings — so this resolves against the
/// override table and leaves anything else alone. A caller that gets back a key
/// the roster does not know should drop the row rather than guess.
pub fn counterwatch_to_ours(slug: &str) -> String {
    COUNTERWATCH
        .iter()
        .find(|(_, theirs)| *theirs == slug)
        .map(|(ours, _)| (*ours).to_owned())
        .unwrap_or_else(|| slug.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_is_tried_before_our_own_spelling() {
        assert_eq!(counterpickgg("freja"), vec!["freya", "freja"]);
    }

    #[test]
    fn hyphenation_is_the_automatic_fallback() {
        assert_eq!(
            counterwatch("wrecking-ball"),
            vec!["wrecking-ball", "wreckingball"]
        );
    }

    #[test]
    fn an_override_and_a_fallback_can_coexist() {
        assert_eq!(
            counterwatch("soldier-76"),
            vec!["soldier76", "soldier-76"],
            "the known-good spelling is tried first"
        );
    }

    #[test]
    fn unremarkable_keys_produce_a_single_candidate() {
        assert_eq!(counterpickgg("reinhardt"), vec!["reinhardt"]);
    }

    #[test]
    fn opponent_slugs_map_back_to_our_keys() {
        assert_eq!(counterpickgg_to_ours("freya"), "freja");
        assert_eq!(counterpickgg_to_ours("reinhardt"), "reinhardt");
    }

    #[test]
    fn partner_slugs_map_back_to_our_keys() {
        assert_eq!(counterwatch_to_ours("jetpackcat"), "jetpack-cat");
        assert_eq!(counterwatch_to_ours("soldier76"), "soldier-76");
        // Already ours, and hyphenated on their side too.
        assert_eq!(counterwatch_to_ours("wrecking-ball"), "wrecking-ball");
        assert_eq!(counterwatch_to_ours("lucio"), "lucio");
    }
}
