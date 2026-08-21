//! Cleaning up after counterpickgg's own templates.
//!
//! The rationale sentences are the site's words, quoted exactly, and they are the
//! best prose in the dataset. But 130 of the 1,066 committed rows carry residue
//! the site meant to resolve and did not, and on screen it is indistinguishable
//! from the sentences that came out right. Four classes, as committed:
//!
//! - **40 rows** hold an unresolved keybind placeholder — `Freja's Freja:R-CLICK`
//!   — eight distinct placeholders across twenty sentences, all four heroes 2026
//!   releases whose ability names the site's template never filled in.
//! - **72 rows** spell a hero without its accents: `Lucio` thirty times,
//!   `Torbjorn` forty-two. The roster spells them `Lúcio` and `Torbjörn`, and so
//!   does the site itself on four of the Torbjörn rows — it is inconsistent
//!   rather than wrong, which is why the fix is a restoration and not a rename.
//! - **16 rows** open on a pronoun with no antecedent, because the sentence that
//!   introduced the hero is on a part of the page we do not read.
//! - **2 rows** carry a footnote marker and a stale claim behind it.
//!
//! Checked for and absent: URLs, HTML entities, non-breaking spaces, double
//! spaces, `Note:`, second person, HTML tags, ellipses, lowercase starts, missing
//! terminal periods.
//!
//! **Fixed here rather than at render.** The diff review is the curation step, so
//! a change to the words belongs in a diff; a render-time fix would cost wasm
//! bytes, re-derive itself on every read, and leave the artefact in the committed
//! file forever.
//!
//! **What this does not do.** It does not rewrite, retemplate, capitalise or
//! truncate a sentence. Every output is the site's own wording with residue
//! removed, which is what keeps the text quotable and attributable to them.

use std::collections::HashMap;

use anyhow::Result;
use overwatch_data::schema::MatchupsFile;

/// What each unresolved placeholder stands for, as `(placeholder, ability)`.
///
/// The one table here with no source in this repository. counterpickgg failed to
/// resolve these itself — the abilities it *does* resolve arrive as tooltip spans
/// carrying the name, and these arrive as bare text — so the names come from
/// outside, and each one is also checked against the sentence it appears in:
///
/// - `Domina:L-CLICK` is "hitscan, making it easy to hit airborne targets", and
///   Photon Magnum is the beam that culminates in a hitscan shot.
/// - `Domina:R-CLICK` leaves her "vulnerable once the shield is destroyed" and
///   "becomes a charge resource for Symmetra's primary fire" — a barrier.
/// - `Freja:R-CLICK` is something her "primary fire during" happens inside, so a
///   state rather than a weapon. Take Aim slows her momentum to charge a bolt.
/// - `Freja:E` is used "against Boosters engages": the vertical escape. Quick
///   Dash is her Shift, Updraft her E.
/// - `Emre:Shift` is what he is "slow" without and vulnerable "even during", and
///   Siphon Blaster is the pistol he moves faster and jumps higher while holding.
/// - `Emre:E` punishes a Mei who "lacks mobility" — a grenade that bounces first.
/// - `Mizuki:Shift` is how she "dodges" a committed engage: the leap.
/// - `Mizuki:E` "negates Doomfist's punch", which is what hindering does — a
///   hindered enemy cannot use a movement ability, and Rocket Punch is one.
///
/// Longest first, so a placeholder that is a prefix of another could never
/// resolve to the shorter one. Nothing in the table collides today; the ordering
/// costs nothing and stops the next entry having to think about it.
const KEYBINDS: &[(&str, &str)] = &[
    ("Domina:L-CLICK", "Photon Magnum"),
    ("Domina:R-CLICK", "Barrier Array"),
    ("Freja:R-CLICK", "Take Aim"),
    ("Mizuki:Shift", "Katashiro Return"),
    ("Emre:Shift", "Siphon Blaster"),
    ("Mizuki:E", "Binding Chain"),
    ("Freja:E", "Updraft"),
    ("Emre:E", "Cyber Frag"),
];

/// The key names the site's placeholders end in, longest first.
///
/// Used only to *detect* a placeholder [`KEYBINDS`] does not cover, so a new
/// hero's unresolved abilities stop the run instead of shipping. Every other
/// colon in the committed prose is followed by a space — `Soldier: 76`,
/// `Configuration: Assault` — and a placeholder's never is, which is the whole
/// discriminator and the reason this cannot fire on a hero's own name.
const KEYS: &[&str] = &["R-CLICK", "L-CLICK", "Shift", "E", "Q"];

/// Sentence-initial pronouns, as `(word, possessive)`, longest first so `He`
/// cannot shadow `Her`.
const PRONOUNS: &[(&str, bool)] = &[
    ("Their", true),
    ("They", false),
    ("Her", true),
    ("His", true),
    ("Its", true),
    ("She", false),
    ("He", false),
    ("It", false),
];

/// What a cleaning pass changed, for the run's own report.
///
/// Per class rather than a single total, because the four have very different
/// blast radii and a number that moved unexpectedly should say which cleaner
/// moved it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProseReport {
    pub rows_changed: usize,
    pub keybinds: usize,
    pub spellings: usize,
    pub pronouns: usize,
    pub footnotes: usize,
}

impl ProseReport {
    pub fn render(&self) -> String {
        if self.rows_changed == 0 {
            return "  prose: nothing to clean".to_owned();
        }
        format!(
            "  prose: {} row(s) cleaned - {} keybind, {} spelling, {} pronoun, {} footnote",
            self.rows_changed, self.keybinds, self.spellings, self.pronouns, self.footnotes
        )
    }
}

/// Cleans every rationale in the file, in place.
///
/// `names` is hero key to display name. Runs after `merge_matchups` and before
/// serialisation, on both write paths, because either one rebuilds `reason` from
/// the sources and a cleaner wired into only one of them would regress through
/// the other.
///
/// The curated `note` column is deliberately untouched: that text is ours, and
/// none of these artefacts can appear in it.
pub fn clean_file(file: &mut MatchupsFile, names: &HashMap<String, String>) -> Result<ProseReport> {
    let spellings = spelling_table(names);
    let mut report = ProseReport::default();

    for entry in &mut file.matchups {
        if entry.reason.is_empty() {
            continue;
        }
        let cleaned = clean(
            &entry.reason,
            &entry.hero,
            &entry.vs,
            names,
            &spellings,
            &mut report,
        )?;
        if cleaned != entry.reason {
            report.rows_changed += 1;
            entry.reason = cleaned;
        }
    }

    Ok(report)
}

/// One sentence, cleaned.
///
/// Order matters twice. Spellings run before the pronoun pass, because that pass
/// matches display names inside the sentence and an unaccented `Lucio` would not
/// be recognised as one. Keybinds run before it for the same reason: a
/// placeholder carries a hero's name and has no business being read as a mention
/// of them. The footnote goes first so nothing else processes text about to be
/// dropped.
///
/// Takes the report rather than returning counts, so a caller cannot forget to
/// accumulate one of the four.
fn clean(
    reason: &str,
    hero: &str,
    vs: &str,
    names: &HashMap<String, String>,
    spellings: &[(String, String)],
    report: &mut ProseReport,
) -> Result<String> {
    let stripped = strip_footnote(reason);
    if stripped != reason {
        report.footnotes += 1;
    }

    let resolved = resolve_keybinds(&stripped, hero, vs)?;
    if resolved != stripped {
        report.keybinds += 1;
    }

    let respelled = restore_spellings(&resolved, spellings);
    if respelled != resolved {
        report.spellings += 1;
    }

    let named = resolve_leading_pronoun(&respelled, hero, vs, names);
    if named != respelled {
        report.pronouns += 1;
    }

    Ok(named)
}

/// Drops a footnote marker and the clause behind it.
///
/// The clause goes with the glyph rather than only the glyph, because the one
/// committed occurrence is a *stale* claim — "Before the update, Anran could
/// cleanse Nade, but this was removed in an update" — and a sentence about a
/// mechanic the game no longer has is worse than no sentence. A marker with
/// something still true behind it has never appeared; if one does, this is the
/// function that has to learn the difference.
fn strip_footnote(reason: &str) -> String {
    match reason.find('*') {
        Some(at) => reason[..at].trim_end().to_owned(),
        None => reason.to_owned(),
    }
}

/// Substitutes the ability each placeholder stands for, and refuses to ship one
/// it does not know.
///
/// The error rather than a warning is the whole safety of [`KEYBINDS`]: the table
/// is hand-written from outside this repository, so the failure mode worth
/// designing for is a new hero arriving with placeholders nobody has looked up.
/// A warning would scroll past.
fn resolve_keybinds(reason: &str, hero: &str, vs: &str) -> Result<String> {
    let mut out = reason.to_owned();
    for (placeholder, ability) in KEYBINDS {
        if out.contains(placeholder) {
            out = out.replace(placeholder, ability);
        }
    }

    if let Some(unknown) = residual_placeholder(&out) {
        anyhow::bail!(
            "{hero} vs {vs} carries the unresolved placeholder {unknown:?} - \
             look the ability up and add it to KEYBINDS in overwatch-ingest/src/prose.rs \
             rather than letting it reach data/matchups.toml"
        );
    }

    Ok(out)
}

/// The first `Name:KEY` left in the text, if any.
///
/// A key token has to end the word it is in, so `Configuration: Assault` and
/// `Soldier: 76` — which put a space after the colon — cannot reach the check,
/// and neither could ordinary prose that happened to end a clause in a colon.
fn residual_placeholder(text: &str) -> Option<String> {
    for (at, _) in text.match_indices(':') {
        let after = &text[at + 1..];
        for key in KEYS {
            let Some(rest) = after.strip_prefix(*key) else {
                continue;
            };
            if rest.starts_with(|c: char| c.is_alphanumeric() || c == '-') {
                continue;
            }
            let before = &text[..at];
            let start = before
                .rfind(|c: char| !(c.is_alphanumeric() || c == '.' || c == '-'))
                .map_or(0, |boundary| boundary + 1);
            // A bare colon is punctuation. Only a colon bound to a word in front
            // of it is a placeholder.
            if start == at {
                continue;
            }
            return Some(format!("{}:{}", &before[start..], key));
        }
    }
    None
}

/// The accented spellings the roster uses, keyed by the unaccented form the site
/// sometimes writes instead.
///
/// Derived from the roster rather than tabulated, so the next accented release is
/// handled without an edit here. Two entries today.
fn spelling_table(names: &HashMap<String, String>) -> Vec<(String, String)> {
    let mut table: Vec<(String, String)> = names
        .values()
        .map(|name| (deaccent(name), name.clone()))
        .filter(|(folded, name)| folded != name)
        .collect();
    // The map's iteration order is arbitrary and the report counts substitutions,
    // so fix an order or two runs can disagree about nothing.
    table.sort();
    table
}

/// Strips the accents from a name while leaving everything else alone.
///
/// [`overwatch_core`] has a sibling of this in `search.rs`, and it is the wrong
/// shape to reuse twice over: it is private, and it lowercases and drops
/// punctuation because it builds a comparison key. What is needed here is a
/// *spelling* — `Lúcio` has to become exactly `Lucio` so it can be found and put
/// back, not `lucio`.
fn deaccent(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'É' | 'È' | 'Ê' | 'Ë' => 'E',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'Í' | 'Ì' | 'Î' | 'Ï' => 'I',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'O',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'Ú' | 'Ù' | 'Û' | 'Ü' => 'U',
            'ñ' => 'n',
            'Ñ' => 'N',
            'ç' => 'c',
            'Ç' => 'C',
            other => other,
        })
        .collect()
}

/// Puts the roster's spelling back wherever the site dropped the accents.
fn restore_spellings(reason: &str, spellings: &[(String, String)]) -> String {
    let mut out = reason.to_owned();
    for (folded, canonical) in spellings {
        out = replace_word(&out, folded, canonical);
    }
    out
}

/// `str::replace` that will not fire inside a longer word.
///
/// No two hero names on the current roster contain one another, so a plain
/// replace would do — but a release named for a prefix of another would corrupt
/// prose silently, and a possessive or a plural is one character away from
/// looking like the same case.
fn replace_word(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(from) {
        let before_ok = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after = &rest[at + from.len()..];
        let after_ok = after.chars().next().is_none_or(|c| !c.is_alphanumeric());

        out.push_str(&rest[..at]);
        if before_ok && after_ok {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Replaces a sentence-initial pronoun with the hero it must mean.
///
/// The site's card opens on a pronoun whose antecedent is elsewhere on the page,
/// so the sentence arrives without it. What identifies the hero is that the
/// clause names *the other* member of the pair: "Her attacks are easily absorbed
/// by D.Va's DM" is on a D.Va card, so "Her" is whoever D.Va is being read
/// against.
///
/// **Keyed on the unordered pair, and that is load-bearing.** counterpickgg
/// publishes a pair's card on both heroes' pages, so `(a,b)` and `(b,a)` hold
/// byte-identical text, and nothing in this repository mirrors `reason` — the two
/// rows are independently scraped copies. A rule phrased as "the subject" would
/// answer differently on each and split every one of these pairs.
///
/// Bounded to the *first* clause for the same reason it needs bounding at all:
/// over the whole sentence, "Even if Ashe uses Coach Gun…" names Ashe as well as
/// D.Va, and two names is no answer. Left exactly as it came whenever the clause
/// names both members or neither.
fn resolve_leading_pronoun(
    reason: &str,
    hero: &str,
    vs: &str,
    names: &HashMap<String, String>,
) -> String {
    let Some((pronoun, possessive)) = PRONOUNS
        .iter()
        .find(|(word, _)| reason.starts_with(&format!("{word} ")))
    else {
        return reason.to_owned();
    };

    let Some(subject) = names.get(hero) else {
        return reason.to_owned();
    };
    let Some(opponent) = names.get(vs) else {
        return reason.to_owned();
    };

    let clause = match reason.find(". ") {
        Some(end) => &reason[..end + 1],
        None => reason,
    };

    let antecedent = match (
        mentions(clause, subject),
        mentions(clause, opponent),
        subject == opponent,
    ) {
        // The mirror names itself and nobody else, so the pronoun is the other.
        (true, false, false) => opponent,
        (false, true, false) => subject,
        // Both named, neither named, or the mirror row of a hero against itself.
        _ => return reason.to_owned(),
    };

    let replacement = if *possessive {
        format!("{antecedent}'s")
    } else {
        antecedent.clone()
    };
    format!("{replacement}{}", &reason[pronoun.len()..])
}

/// Whether a clause names a hero, on word boundaries so `Ana` is not found
/// inside a longer name and a possessive still counts.
fn mentions(clause: &str, name: &str) -> bool {
    clause.match_indices(name).any(|(at, _)| {
        let before_ok = clause[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        let after_ok = clause[at + name.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric());
        before_ok && after_ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> HashMap<String, String> {
        [
            ("ana", "Ana"),
            ("anran", "Anran"),
            ("dva", "D.Va"),
            ("echo", "Echo"),
            ("hanzo", "Hanzo"),
            ("lucio", "Lúcio"),
            ("mizuki", "Mizuki"),
            ("soldier-76", "Soldier: 76"),
            ("torbjorn", "Torbjörn"),
        ]
        .into_iter()
        .map(|(key, name)| (key.to_owned(), name.to_owned()))
        .collect()
    }

    fn cleaned(reason: &str, hero: &str, vs: &str) -> String {
        let names = roster();
        let spellings = spelling_table(&names);
        let mut report = ProseReport::default();
        clean(reason, hero, vs, &names, &spellings, &mut report).expect("no unknown placeholder")
    }

    #[test]
    fn a_footnote_marker_takes_the_stale_footnote_with_it() {
        assert_eq!(
            strip_footnote(
                "Anran can dodge Ana's Nade and Sleep with high mobility. \
                 *Before the update, Anran could cleanse Nade, but this was removed in an update."
            ),
            "Anran can dodge Ana's Nade and Sleep with high mobility."
        );
    }

    #[test]
    fn an_unresolved_keybind_placeholder_is_replaced_with_the_ability_it_stands_for() {
        assert_eq!(
            resolve_keybinds(
                "Mizuki can negate Doomfist's punch with Mizuki:E or dodge it with Mizuki:Shift.",
                "doomfist",
                "mizuki"
            )
            .expect("both are in the table"),
            "Mizuki can negate Doomfist's punch with Binding Chain or dodge it with Katashiro Return."
        );
        // The site writes the possessive and the placeholder, so the hero's name
        // is already there and the substitution has to read as prose beside it.
        assert_eq!(
            resolve_keybinds("Freja's long Freja:R-CLICK range.", "freja", "roadhog")
                .expect("in the table"),
            "Freja's long Take Aim range."
        );
    }

    #[test]
    fn an_unknown_keybind_placeholder_stops_the_ingest_rather_than_shipping_it() {
        let err = resolve_keybinds("Newhero can escape with Newhero:Shift.", "newhero", "ana")
            .expect_err("an unmapped placeholder must not ship");
        let message = format!("{err:#}");
        assert!(message.contains("Newhero:Shift"), "{message}");
        assert!(message.contains("KEYBINDS"), "unhelpful error: {message}");
    }

    /// Two colons in the committed prose belong to hero and ability names, and
    /// both put a space after the colon where a placeholder never does. Without
    /// that discriminator this check would fail 24 rows about Soldier: 76.
    #[test]
    fn the_roster_spelling_of_soldier_76_is_not_mistaken_for_a_keybind() {
        assert_eq!(residual_placeholder("Soldier: 76's primary fire."), None);
        assert_eq!(
            residual_placeholder("Bastion's Configuration: Assault is a problem."),
            None
        );
        // Nor is a colon that ends a clause.
        assert_eq!(residual_placeholder("One thing: Everything else."), None);
    }

    #[test]
    fn an_ascii_folded_hero_name_is_restored_to_the_spelling_the_roster_uses() {
        assert_eq!(
            cleaned("Torbjorn's turret punishes Lucio.", "torbjorn", "lucio"),
            "Torbjörn's turret punishes Lúcio."
        );
        // The site spells it correctly on some rows already, and those must come
        // through untouched rather than double-substituted.
        assert_eq!(
            cleaned("Torbjörn's turret melts him.", "torbjorn", "ana"),
            "Torbjörn's turret melts him."
        );
    }

    #[test]
    fn a_leading_pronoun_becomes_the_hero_its_own_clause_does_not_name() {
        assert_eq!(
            cleaned(
                "Her attacks are easily absorbed by D.Va's DM. Ana has great difficulty \
                 dealing with Boosters engages.",
                "ana",
                "dva"
            ),
            "Ana's attacks are easily absorbed by D.Va's DM. Ana has great difficulty \
             dealing with Boosters engages."
        );
        // The clause, not the sentence: the second half names Ashe as well, and
        // over the whole string there would be two candidates and no answer.
        assert_eq!(
            cleaned(
                "His attacks are easily absorbed by D.Va's DM. Hanzo has difficulty \
                 dealing with Boosters engages.",
                "dva",
                "hanzo"
            ),
            "Hanzo's attacks are easily absorbed by D.Va's DM. Hanzo has difficulty \
             dealing with Boosters engages."
        );
    }

    /// The trap. Both rows of a pair hold the same text, scraped independently
    /// from the two hero pages, and nothing mirrors `reason` afterwards - so a
    /// rule that read the subject would resolve this row one way and its mirror
    /// the other, and the pair would stop agreeing with itself.
    #[test]
    fn a_pronoun_resolves_the_same_way_from_either_side_of_the_pair() {
        let text = "Her attacks are easily absorbed by D.Va's DM. D.Va can also commit \
                    to airborne targets.";
        let forward = cleaned(text, "dva", "echo");
        let reverse = cleaned(text, "echo", "dva");

        assert_eq!(forward, reverse);
        assert!(
            forward.starts_with("Echo's attacks"),
            "the clause names D.Va, so the pronoun is Echo: {forward}"
        );
    }

    #[test]
    fn a_leading_pronoun_with_nothing_to_resolve_against_is_left_exactly_as_it_came() {
        // Neither hero named in the clause.
        let neither = "Her attacks bounce off the barrier. Ana struggles here.";
        assert_eq!(cleaned(neither, "ana", "dva"), neither);

        // Both named, so there is no "the other one".
        let both = "Her attacks are absorbed by D.Va's DM even when Ana lands them.";
        assert_eq!(cleaned(both, "ana", "dva"), both);

        // A hero the roster cannot name.
        let unknown = "Her attacks are easily absorbed by D.Va's DM.";
        assert_eq!(cleaned(unknown, "nobody", "dva"), unknown);
    }

    #[test]
    fn cleaning_a_sentence_with_nothing_wrong_with_it_changes_nothing() {
        let fine = "Reinhardt cannot reach an air angle, so Pharah farms him from above.";
        assert_eq!(cleaned(fine, "reinhardt", "pharah"), fine);

        // And a pronoun that is not sentence-initial is somebody else's problem.
        let mid = "Ana cannot follow him, so her Nade lands late.";
        assert_eq!(cleaned(mid, "ana", "dva"), mid);
    }

    #[test]
    fn the_report_counts_each_class_separately() {
        let names = roster();
        let spellings = spelling_table(&names);
        let mut report = ProseReport::default();

        clean(
            "Her attacks are absorbed by D.Va's DM. Lucio cannot keep up. \
             *Before the update this was different.",
            "lucio",
            "dva",
            &names,
            &spellings,
            &mut report,
        )
        .expect("no placeholder here");

        assert_eq!(report.footnotes, 1);
        assert_eq!(report.spellings, 1);
        assert_eq!(report.pronouns, 1);
        assert_eq!(report.keybinds, 0);
    }

    #[test]
    fn the_spelling_table_is_built_from_the_roster_rather_than_written_out() {
        let table = spelling_table(&roster());
        assert_eq!(
            table,
            vec![
                ("Lucio".to_owned(), "Lúcio".to_owned()),
                ("Torbjorn".to_owned(), "Torbjörn".to_owned()),
            ],
            "only the names that actually carry an accent, in a fixed order"
        );
    }
}
