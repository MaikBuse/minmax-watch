//! Invariants the committed dataset in `data/` must hold.
//!
//! These run against the real generated files, so they are the guard that
//! catches a bad ingest before it reaches the draft screen. A scraper that
//! silently starts returning nothing, or that transposes the matrix, fails
//! here rather than in the middle of a hero select.

use overwatch_core::{
    ban_recommendations, recommend, threats, BanSubject, Defended, DefendedTeam, Draft, HeroId,
    Knowledge, MapId, Role, UserContext,
};
use overwatch_data::load;
use overwatch_data::schema::MatchupsFile;

#[test]
fn the_roster_is_plausible() {
    let ds = load().expect("committed data must load");

    assert!(
        ds.hero_count() >= 40,
        "roster collapsed to {} heroes",
        ds.hero_count()
    );
    for role in [Role::Tank, Role::Damage, Role::Support] {
        let count = ds.heroes_in_role(role).count();
        assert!(count >= 10, "{role:?} has only {count} heroes");
    }
    assert!(!ds.maps().is_empty(), "no maps");
}

#[test]
fn every_hero_has_a_short_form_to_type() {
    let ds = load().expect("committed data must load");

    for hero in ds.heroes() {
        assert!(
            !hero.aliases.is_empty(),
            "{} has no aliases, so it cannot be typed quickly",
            hero.key
        );
    }
}

/// Coverage is counted from the entries the ingest wrote, not from non-zero cells
/// in the matrix. Zero is a perfectly ordinary matchup value — it is what the
/// primary source's neutral rating converts to, and it covers a quarter of the
/// file — so "cell is zero" says nothing about whether anyone rated the pair.
#[test]
fn the_matrix_is_broadly_populated() {
    let ds = load().expect("committed data must load");
    let matchups: MatchupsFile =
        toml::from_str(overwatch_data::MATCHUPS_TOML).expect("committed matchups must parse");

    let n = ds.hero_count();
    let off_diagonal = n * (n - 1);
    let rated = matchups.matchups.len();

    // The newest heroes genuinely have no community data yet, so this asserts
    // broad coverage rather than completeness.
    assert!(
        rated * 10 >= off_diagonal * 8,
        "only {rated}/{off_diagonal} pairs are rated"
    );
}

/// Matchups every Overwatch player knows. If the pipeline transposes the matrix
/// or flips a sign, these are what catch it.
#[test]
fn known_matchups_have_the_right_sign() {
    let ds = load().expect("committed data must load");

    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };

    let reinhardt = hero("reinhardt");
    let pharah = hero("pharah");
    let brigitte = hero("brigitte");
    let dva = hero("dva");
    let winston = hero("winston");
    let widowmaker = hero("widowmaker");

    // Reinhardt cannot touch a hero in the air.
    assert!(
        ds.matchups().get(reinhardt, pharah) < 0,
        "Reinhardt should lose to Pharah"
    );
    assert!(
        ds.matchups().get(pharah, reinhardt) > 0,
        "and Pharah should beat Reinhardt"
    );
    // Reinhardt out-ranges and out-damages Brigitte at close quarters.
    assert!(
        ds.matchups().get(reinhardt, brigitte) > 0,
        "Reinhardt should beat Brigitte"
    );
    // Defense Matrix eats rockets.
    assert!(
        ds.matchups().get(dva, pharah) > 0,
        "D.Va should beat Pharah"
    );
    // Winston dives the sniper.
    assert!(
        ds.matchups().get(widowmaker, winston) < 0,
        "Widowmaker should lose to Winston"
    );
}

#[test]
fn rationale_text_is_attached_to_real_matchups() {
    let ds = load().expect("committed data must load");

    let reinhardt = ds.hero_by_key("reinhardt").expect("present");
    let pharah = ds.hero_by_key("pharah").expect("present");

    let reason = ds
        .reason(reinhardt, pharah)
        .expect("the Reinhardt/Pharah matchup should carry an explanation");
    assert!(
        reason.to_lowercase().contains("air"),
        "unexpected rationale: {reason:?}"
    );
}

#[test]
fn base_strength_and_map_affinity_are_populated() {
    let ds = load().expect("committed data must load");

    let mut rated = 0;
    for i in 0..ds.hero_count() {
        if ds.base_strength(HeroId(i as u16)) != 0 {
            rated += 1;
        }
    }
    assert!(rated > 20, "only {rated} heroes have a strength value");

    let mut affinity = 0;
    for m in 0..ds.maps().len() {
        for h in 0..ds.hero_count() {
            if ds.map_affinity(MapId(m as u16), HeroId(h as u16)) != 0 {
                affinity += 1;
            }
        }
    }
    assert!(affinity > 50, "only {affinity} hero/map affinities set");
}

/// The end-to-end check: real data, a real draft, a sane answer.
#[test]
fn a_real_draft_produces_a_sane_tank_recommendation() {
    let ds = load().expect("committed data must load");
    let ctx = UserContext::new(Role::Tank, ds.hero_count());

    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };
    let reinhardt = hero("reinhardt");
    let dva = hero("dva");
    let winston = hero("winston");

    // A classic air comp: the answer must not be Reinhardt.
    let mut draft = Draft::new();
    draft.add_enemy(hero("pharah"));
    draft.add_enemy(hero("echo"));

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    assert_eq!(
        recs.len(),
        ds.heroes_in_role(Role::Tank).count(),
        "every tank is a candidate — nothing narrows the role"
    );

    let rank_of = |target| {
        recs.iter()
            .position(|r| r.hero == target)
            .expect("every tank is ranked")
    };
    assert!(
        rank_of(reinhardt) > rank_of(dva) && rank_of(reinhardt) > rank_of(winston),
        "Reinhardt should rank below the dive answers into an air comp"
    );

    let best = &recs[0];
    assert!(
        !best.reasons.is_empty(),
        "the top pick must come with an explanation"
    );
}

#[test]
fn the_threat_board_identifies_the_real_problem() {
    let ds = load().expect("committed data must load");
    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };

    let mut draft = Draft::new();
    draft.add_enemy(hero("brigitte"));
    draft.add_enemy(hero("pharah"));

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let board = threats(&ds, &draft, &ctx, hero("reinhardt"));

    assert_eq!(board.len(), 2);
    assert_eq!(
        board[0].enemy,
        hero("pharah"),
        "Pharah, not Brigitte, is what is beating Reinhardt"
    );
    assert!(board[0].severity > 0.0);
}

#[test]
fn support_mode_offers_only_supports() {
    let ds = load().expect("committed data must load");
    let ctx = UserContext::new(Role::Support, ds.hero_count());

    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");

    for rec in &recs {
        let hero = ds.hero(rec.hero).expect("ranked hero exists");
        assert_eq!(hero.role, Role::Support, "{} is not a support", hero.key);
    }
}

#[test]
fn damage_mode_offers_only_damage() {
    let ds = load().expect("committed data must load");
    let ctx = UserContext::new(Role::Damage, ds.hero_count());

    let recs = recommend(&ds, &Draft::new(), &ctx).expect("scoring succeeds");

    assert!(!recs.is_empty(), "damage mode ranked nobody");
    for rec in &recs {
        let hero = ds.hero(rec.hero).expect("ranked hero exists");
        assert_eq!(hero.role, Role::Damage, "{} is not a damage hero", hero.key);
    }
}

/// The damage mode leans on the same committed matchup rows as the other two.
/// A scraper that quietly stopped covering damage heroes would leave the mode
/// ranking on base strength alone, which reads as a working list rather than as
/// the silence it is.
#[test]
fn the_damage_roster_has_matchup_data_to_rank_on() {
    let ds = load().expect("committed data must load");

    let mut draft = Draft::new();
    draft.add_enemy(
        ds.hero_by_key("reinhardt")
            .expect("Reinhardt is on the roster"),
    );
    draft.add_enemy(ds.hero_by_key("ana").expect("Ana is on the roster"));

    let ctx = UserContext::new(Role::Damage, ds.hero_count());
    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");

    let rated = recs
        .iter()
        .filter(|rec| {
            rec.reasons.iter().any(|reason| {
                matches!(
                    reason.kind,
                    overwatch_core::ReasonKind::BeatsEnemy(_)
                        | overwatch_core::ReasonKind::LosesToEnemy(_)
                )
            })
        })
        .count();

    assert!(
        rated >= 20,
        "only {rated} damage heroes have a reading against Reinhardt or Ana"
    );
}

/// The guard against `side.toml` going the way of `synergy.toml`, which is
/// committed empty and therefore contributes nothing to any score. A curated
/// file only earns its weight in the scorer while it actually has values in it.
#[test]
fn the_attack_defend_leans_are_populated() {
    let ds = load().expect("committed data must load");

    let leaning = (0..ds.hero_count())
        .map(|i| HeroId(i as u16))
        .filter(|hero| ds.side_lean(*hero) != 0)
        .count();
    assert!(
        leaning >= 10,
        "only {leaning} heroes have an attack/defend lean - side.toml is going stale"
    );

    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };
    // Signs, not magnitudes: a transposed file would still pass a count.
    assert!(
        ds.side_lean(hero("bastion")) < 0,
        "Bastion is a defence pick"
    );
    assert!(
        ds.side_lean(hero("winston")) > 0,
        "Winston is an attack pick"
    );
}

/// The ban list against real data, in the state it is actually used in: nobody
/// has picked yet, so the answer has to come from the pool alone.
#[test]
fn the_ban_list_answers_for_a_pool_before_anyone_picks() {
    let ds = load().expect("committed data must load");
    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };

    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let alone = |knowledge, heroes| DefendedTeam {
        members: vec![Defended {
            who: "me".to_owned(),
            is_me: true,
            is_typed: false,
            role: Role::Tank,
            knowledge,
            heroes,
        }],
    };
    let pool = alone(Knowledge::Pool, vec![hero("reinhardt"), hero("winston")]);

    let board = ban_recommendations(&ds, &Draft::new(), &ctx, &pool);

    assert_eq!(
        board.subject,
        BanSubject::One {
            who: "me".to_owned(),
            is_me: true,
            locked: false,
            heroes: 2,
        }
    );
    assert!(
        board.candidates.len() >= 10,
        "only {} heroes rate as a threat to a two-hero pool",
        board.candidates.len()
    );
    // Sorted, and every entry is a hero that actually beats you — a list that
    // ran past zero would be padding itself with heroes you already beat.
    for pair in board.candidates.windows(2) {
        assert!(pair[0].score >= pair[1].score, "candidates are not sorted");
    }
    assert!(board.candidates.iter().all(|c| c.score > 0.0));

    // Ramattra pierces Reinhardt's barrier and Orisa stops his charge; both are
    // the enemy tank, which for a tank player is the pick that decides the game.
    let top: Vec<_> = board.candidates[..3]
        .iter()
        .map(|c| ds.hero(c.hero).expect("ranked hero exists").key.as_str())
        .collect();
    assert!(
        top.contains(&"ramattra") && top.contains(&"orisa"),
        "unexpected top bans for a Reinhardt/Winston pool: {top:?}"
    );
    assert!(
        board.candidates[0].worst == Some(hero("reinhardt"))
            || board.candidates[0].worst == Some(hero("winston")),
        "the worst case has to be one of the heroes being defended"
    );
    assert!(
        board
            .candidates
            .iter()
            .all(|c| c.hero != hero("reinhardt") && c.hero != hero("winston")),
        "a ban takes the hero from you too, so your own pool is never on the list"
    );
    assert!(
        !board.candidates[0].text.is_empty(),
        "the top ban must come with an explanation"
    );

    // Locking in narrows the same question to one hero, and the answer moves
    // with it: Winston's problems stop counting once you are on Reinhardt.
    let mut draft = Draft::new();
    draft.locked = Some(hero("reinhardt"));
    let locked = ban_recommendations(
        &ds,
        &draft,
        &ctx,
        &alone(
            Knowledge::Locked(hero("reinhardt")),
            vec![hero("reinhardt")],
        ),
    );

    assert_eq!(
        locked.subject,
        BanSubject::One {
            who: "me".to_owned(),
            is_me: true,
            locked: true,
            heroes: 1,
        }
    );
    assert!(locked
        .candidates
        .iter()
        .all(|c| c.worst == Some(hero("reinhardt"))));
    assert!(
        locked.candidates.iter().any(|c| c.hero == hero("pharah")),
        "Reinhardt cannot touch a hero in the air"
    );
}
