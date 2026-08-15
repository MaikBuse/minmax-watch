//! Does the matcher actually make picks fast against the real 53-hero roster?
//!
//! These are the acceptance tests for the input half of the speed requirement.
//! A hero select gives you seconds, so what matters is not that a query
//! eventually finds the hero but that the *first* result is right after very
//! few keystrokes.

use overwatch_core::{resolve, search, HeroSet, Role, Scope};
use overwatch_data::load;

/// Every hero must be reachable from the shorthand a player would actually type.
#[test]
fn common_shorthand_resolves_to_the_right_hero() {
    let ds = load().expect("committed data must load");
    let scope = Scope::any();

    let cases = [
        ("rein", "reinhardt"),
        ("rh", "reinhardt"),
        ("hog", "roadhog"),
        ("ball", "wrecking-ball"),
        ("jq", "junker-queen"),
        ("queen", "junker-queen"),
        ("dva", "dva"),
        ("sig", "sigma"),
        ("zen", "zenyatta"),
        ("bap", "baptiste"),
        ("lw", "lifeweaver"),
        ("kiri", "kiriko"),
        ("brig", "brigitte"),
        ("widow", "widowmaker"),
        ("torb", "torbjorn"),
        ("cass", "cassidy"),
        ("doom", "doomfist"),
        ("ram", "ramattra"),
        ("soj", "sojourn"),
        ("76", "soldier-76"),
        ("lucio", "lucio"),
        ("monkey", "winston"),
        ("hammond", "wrecking-ball"),
        ("rat", "junkrat"),
        ("sym", "symmetra"),
        ("cat", "jetpack-cat"),
    ];

    for (query, expected) in cases {
        let hero =
            resolve(&ds, query, &scope).unwrap_or_else(|| panic!("{query:?} resolved to nothing"));
        let key = &ds.hero(hero).expect("resolved hero exists").key;
        assert_eq!(key, expected, "{query:?} resolved to {key}");
    }
}

/// Accents and punctuation must never need typing.
#[test]
fn awkward_names_are_typable_in_plain_ascii() {
    let ds = load().expect("committed data must load");
    let scope = Scope::any();

    for (query, expected) in [
        ("lucio", "lucio"),
        ("torbjorn", "torbjorn"),
        ("dva", "dva"),
        ("soldier76", "soldier-76"),
        ("wreckingball", "wrecking-ball"),
    ] {
        let hero = resolve(&ds, query, &scope).unwrap_or_else(|| panic!("{query:?} found nothing"));
        assert_eq!(&ds.hero(hero).expect("exists").key, expected);
    }
}

/// The headline claim: inside your own pool, two characters should be enough.
#[test]
fn two_keystrokes_are_enough_inside_a_pool() {
    let ds = load().expect("committed data must load");

    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };
    let pool = HeroSet::from_iter_checked([
        hero("reinhardt"),
        hero("sigma"),
        hero("dva"),
        hero("winston"),
        hero("orisa"),
    ])
    .expect("fits");
    let scope = Scope::mine(Role::Tank, pool);

    for (query, expected) in [
        ("re", "reinhardt"),
        ("si", "sigma"),
        ("dv", "dva"),
        ("wi", "winston"),
        ("or", "orisa"),
    ] {
        let hero = resolve(&ds, query, &scope).unwrap_or_else(|| panic!("{query:?} found nothing"));
        let key = &ds.hero(hero).expect("exists").key;
        assert_eq!(key, expected, "{query:?} resolved to {key} inside the pool");
    }
}

/// A curated alias must beat an accidental subsequence hit.
#[test]
fn aliases_outrank_incidental_fuzzy_matches() {
    let ds = load().expect("committed data must load");
    let scope = Scope::any();

    // `rh` is a subsequence of Roadhog too; the curated alias must win.
    let hero = resolve(&ds, "rh", &scope).expect("resolves");
    assert_eq!(&ds.hero(hero).expect("exists").key, "reinhardt");

    // `ball` is Wrecking Ball's alias, though other names contain those letters.
    let hero = resolve(&ds, "ball", &scope).expect("resolves");
    assert_eq!(&ds.hero(hero).expect("exists").key, "wrecking-ball");
}

#[test]
fn the_scope_keeps_out_of_role_heroes_out() {
    let ds = load().expect("committed data must load");
    let scope = Scope::mine(Role::Support, HeroSet::empty());

    // Reinhardt is a tank, so support mode must not offer him however you spell it.
    let results = search(&ds, "rein", &scope, 10);
    for result in &results {
        let hero = ds.hero(result.hero).expect("exists");
        assert_eq!(hero.role, Role::Support, "{} leaked in", hero.key);
    }
}

#[test]
fn already_picked_heroes_drop_out_of_the_candidates() {
    let ds = load().expect("committed data must load");
    let reinhardt = ds.hero_by_key("reinhardt").expect("present");

    let taken = HeroSet::from_iter_checked([reinhardt]).expect("fits");
    let scope = Scope::any().excluding(taken);

    let hero = resolve(&ds, "rein", &scope);
    assert!(
        hero != Some(reinhardt),
        "an already-picked hero must not be offered again"
    );
}

#[test]
fn an_empty_query_lists_the_scope_for_browsing() {
    let ds = load().expect("committed data must load");
    let scope = Scope::mine(Role::Tank, HeroSet::empty());

    let results = search(&ds, "", &scope, 100);
    assert_eq!(results.len(), ds.heroes_in_role(Role::Tank).count());
}

#[test]
fn nonsense_matches_nothing_rather_than_guessing() {
    let ds = load().expect("committed data must load");
    assert!(resolve(&ds, "qqqqzzz", &Scope::any()).is_none());
}

/// The same keystrokes must always give the same first result — a matcher that
/// reorders between frames would be unusable under time pressure.
#[test]
fn results_are_deterministic() {
    let ds = load().expect("committed data must load");
    let scope = Scope::any();

    let first = search(&ds, "s", &scope, 10);
    for _ in 0..5 {
        assert_eq!(search(&ds, "s", &scope, 10), first);
    }
}
