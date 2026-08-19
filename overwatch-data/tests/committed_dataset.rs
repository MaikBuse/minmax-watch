//! Invariants the committed dataset in `data/` must hold.
//!
//! These run against the real generated files, so they are the guard that
//! catches a bad ingest before it reaches the draft screen. A scraper that
//! silently starts returning nothing, or that transposes the matrix, fails
//! here rather than in the middle of a hero select.

use std::collections::HashSet;

use overwatch_core::{
    ban_recommendations, recommend, threats, Archetype, BanSubject, Defended, DefendedTeam, Draft,
    HeroId, Knowledge, MapId, Rank, ReasonKind, Role, Subrole, UserContext,
};
use overwatch_data::load;
use overwatch_data::schema::{BanRateFile, MatchupsFile};

/// The yardstick, read straight from `data/` because it is deliberately not one of
/// the documents `overwatch-data` compiles in. See
/// `the_ban_rate_table_never_reaches_the_bundle`.
const BAN_RATE_TOML: &str = include_str!("../../data/ban_rate.toml");

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

/// The flag the blend writes has to survive the loader, because for a long time
/// it did not: `disagreement` was written into `matchups.toml`, deserialized by
/// `MatchupEntry`, and then read by nothing at all, while the README and the
/// schema both claimed the app said so.
///
/// Counts through `sources_disagree` rather than by re-reading the TOML, so this
/// exercises the path the draft screen uses. The count is asserted as a **band**
/// and not a floor: a scrape that stops flagging anything and a scrape that
/// flags everything both slip past a bare `> 0`, and the second one is the more
/// plausible failure — a source going offline makes every pair look contradicted.
#[test]
fn the_disagreement_flags_reach_the_dataset() {
    let ds = load().expect("committed data must load");

    let winston = ds.hero_by_key("winston").expect("present");
    let zarya = ds.hero_by_key("zarya").expect("present");

    // counterwatch rated Winston into Zarya and said nothing about the mirror, so
    // this pair is what the pair-level verdict was built for: both rows carry the
    // flag now, and both readings were pulled toward even by the same factor.
    assert!(
        ds.sources_disagree(winston, zarya),
        "the sources disagree sharply about Winston into Zarya"
    );
    assert!(
        ds.sources_disagree(zarya, winston),
        "and the mirror has to show the same dispute"
    );

    let n = ds.hero_count();
    let mut flagged = 0;
    for a in 0..n {
        for b in 0..n {
            if a != b && ds.sources_disagree(HeroId(a as u16), HeroId(b as u16)) {
                flagged += 1;
            }
        }
    }

    // 164 of the 2534 directed rows today, from 82 flagged pairs counted twice.
    assert!(
        (50..=500).contains(&flagged),
        "{flagged} directed rows read as disputed, which is outside the band a \
         healthy blend produces"
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

/// The end-to-end proof that the synergy term does something.
///
/// It was dead for the whole life of the file it reads: `synergy.toml` shipped
/// with no entries, so `PairsWithAlly` was a variant the UI could render and
/// never did. This asserts against the real data that an ally the sources have
/// paired somebody with now produces a reason on somebody's row.
#[test]
fn an_ally_produces_a_pairs_well_with_reason_on_real_data() {
    let ds = load().expect("committed data must load");
    let ctx = UserContext::new(Role::Tank, ds.hero_count());
    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };

    let mut draft = Draft::new();
    draft.add_ally(hero("lucio"));

    let recs = recommend(&ds, &draft, &ctx).expect("scoring succeeds");
    let cited: Vec<&str> = recs
        .iter()
        .filter(|rec| {
            rec.reasons
                .iter()
                .any(|r| r.kind == ReasonKind::PairsWithAlly(hero("lucio")))
        })
        .filter_map(|rec| ds.hero(rec.hero).ok())
        .map(|h| h.key.as_str())
        .collect();

    assert!(
        !cited.is_empty(),
        "no tank cites Lúcio as a partner - the synergy term is inert again"
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
/// `synergy.toml` shipped empty for long enough that the scorer carried a
/// weighted term multiplying nothing, and every test passed the whole time
/// because an empty matrix is a *valid* matrix. This is the guard that would
/// have caught it: the file has a scraped source now, and a scrape that starts
/// returning nothing looks exactly like the state this is here to prevent.
#[test]
fn the_pair_synergies_are_populated() {
    let ds = load().expect("committed data must load");
    let file: overwatch_data::schema::SynergyFile =
        toml::from_str(overwatch_data::SYNERGY_TOML).expect("committed synergy must parse");

    assert!(
        file.entries.len() >= 200,
        "only {} synergy pairs - synergy.toml is going stale",
        file.entries.len()
    );

    // Coverage counted from entries rather than non-zero cells, for the same
    // reason the matchup guard does it: the two are not the same question.
    let rated = (0..ds.hero_count())
        .map(|i| HeroId(i as u16))
        .filter(|hero| {
            (0..ds.hero_count()).any(|j| ds.synergy().rating(*hero, HeroId(j as u16)).is_some())
        })
        .count();
    assert!(
        rated * 2 >= ds.hero_count(),
        "only {rated} of {} heroes have any duo partner at all",
        ds.hero_count()
    );

    // The source publishes a top-N per hero, so it is a shortlist of good
    // pairings rather than a full ranking. A file that had gone negative on
    // balance would mean it is being read as something it is not.
    let curated = file.entries.iter().filter(|e| e.curated.is_some()).count();
    let positive = file.entries.iter().filter(|e| e.resolved() > 0).count();
    assert!(
        positive * 2 >= file.entries.len(),
        "only {positive} of {} synergy pairs are positive ({curated} curated) - is the scale inverted?",
        file.entries.len()
    );
}

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

/// Same guard for `archetype.toml`, which the ingest also never writes.
///
/// Coverage matters more here than for the side leans: an unrated hero is left
/// out of its team's shape entirely, so a file that quietly emptied would not
/// make the boards wrong, it would make them silent — which reads as "this team
/// has no shape" rather than as missing data.
#[test]
fn the_playstyle_axes_are_populated() {
    let ds = load().expect("committed data must load");

    let read = (0..ds.hero_count())
        .map(|i| HeroId(i as u16))
        .filter(|hero| ds.shape(*hero) != [0; 3])
        .count();
    assert!(
        read * 2 >= ds.hero_count(),
        "only {read} of {} heroes have playstyle axes - archetype.toml is going stale",
        ds.hero_count()
    );

    let hero = |key: &str| {
        ds.hero_by_key(key)
            .unwrap_or_else(|_| panic!("{key} missing"))
    };
    // Which axis leads, not how much: a file whose three columns were
    // transposed would still pass a count, and would still pass a bounds check.
    let leads = |key: &str| {
        let axes = ds.shape(hero(key));
        Archetype::ALL
            .into_iter()
            .max_by_key(|axis| axes[axis.index()])
            .expect("there are three axes")
    };
    assert_eq!(leads("winston"), Archetype::Dive);
    assert_eq!(leads("widowmaker"), Archetype::Poke);
    assert_eq!(leads("reinhardt"), Archetype::Brawl);
}

/// The one external check available on `archetype.toml`.
///
/// Everything in that file is a judgement call with no source behind it, which
/// makes it the easiest file in `data/` to get quietly wrong. Sub-roles are the
/// one published classification that overlaps it: the roster API says what each
/// hero *is*, and for four of the ten sub-roles that answer implies a shape,
/// because the passive is about how the hero fights rather than how it sustains.
///
/// The other six are deliberately not asserted, and the numbers are why. Against
/// the committed roster, the leading axis agrees with the sub-role for only 4/7
/// specialists, 4/6 stalwarts, 3/5 recons, 3/5 tacticians, 2/5 survivors and 2/4
/// medics. Stalwart is the clearest case: it grants knockback and slow
/// resistance, which Reinhardt wants for a brawl and Sigma wants to hold a poke
/// angle. Asserting those would be asserting a coincidence.
///
/// This is a drift alarm, not a rule. If it fails, the honest fix may well be to
/// add an exemption below rather than to change the file.
#[test]
fn the_sub_roles_that_imply_a_shape_agree_with_the_playstyle_axes() {
    let ds = load().expect("committed data must load");

    // Three disagreements, all defensible, all left in the file rather than
    // bent to fit. Reaper is a Flanker by how he arrives and a brawler by what
    // he does once there — Shadow Step is transport, not an engage. Hazard is
    // an Initiator with Violent Leap, but Spike Guard and the wall want the
    // fight to stay where he landed. Cassidy is not a disagreement at all: his
    // poke and brawl are deliberately equal, so he has no leading axis to check
    // and the tie-break below would invent one.
    const EXEMPT: [&str; 3] = ["reaper", "hazard", "cassidy"];

    let expected = |subrole: Subrole| match subrole {
        Subrole::Initiator | Subrole::Flanker => Some(Archetype::Dive),
        Subrole::Bruiser => Some(Archetype::Brawl),
        Subrole::Sharpshooter => Some(Archetype::Poke),
        _ => None,
    };

    let mut checked = 0;
    for i in 0..ds.hero_count() {
        let id = HeroId(i as u16);
        let hero = ds.hero(id).expect("the roster indexes itself");
        let Some(subrole) = hero.subrole else {
            continue;
        };
        let Some(want) = expected(subrole) else {
            continue;
        };
        if EXEMPT.contains(&hero.key.as_str()) {
            continue;
        }

        let axes = ds.shape(id);
        if axes == [0; 3] {
            continue; // nobody has read this kit yet; the count guard above covers that
        }
        let leads = Archetype::ALL
            .into_iter()
            .max_by_key(|axis| axes[axis.index()])
            .expect("there are three axes");
        assert_eq!(
            leads,
            want,
            "{} is a {} but archetype.toml leads {}",
            hero.key,
            subrole.as_str(),
            leads.as_str(),
        );
        checked += 1;
    }

    // Cheap insurance against the guard passing because it checked nothing —
    // a roster that lost its sub-role column would otherwise look healthy.
    assert!(
        checked >= 15,
        "only {checked} heroes carried a shape-bearing sub-role; is heroes.toml missing the column?"
    );
}

/// A sub-role belongs to exactly one role, so one appearing under another means
/// the upstream taxonomy has been reshaped rather than merely extended.
#[test]
fn every_sub_role_sits_under_the_role_it_belongs_to() {
    let ds = load().expect("committed data must load");

    for hero in ds.heroes() {
        if let Some(subrole) = hero.subrole {
            assert_eq!(
                subrole.role(),
                hero.role,
                "{} is {} but carries the {} sub-role {}",
                hero.key,
                hero.role.as_str(),
                subrole.role().as_str(),
                subrole.as_str(),
            );
        }
    }
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

/// The `the_pair_synergies_are_populated` lesson, applied to the newest file.
///
/// Every way of getting the rank fetch wrong produces a file that parses, loads
/// and scores: a cache key without its tier serves one response nine times, a
/// source that quietly falls back to its aggregate does the same, and a smoothing
/// bug that averages everything to the mean does it too. All three ship eight
/// identical columns, and only a test that looks for *variation* catches any of
/// them. Coverage counts alone would pass every one.
#[test]
fn the_rank_shifts_are_populated_and_actually_differ_from_one_another() {
    let ds = load().expect("committed data must load");
    let file: overwatch_data::schema::StrengthByRankFile =
        toml::from_str(overwatch_data::STRENGTH_BY_RANK_TOML)
            .expect("committed rank slices must parse");

    assert!(
        file.entries.len() * 10 >= ds.hero_count() * 8,
        "only {} of {} heroes have a rank row",
        file.entries.len(),
        ds.hero_count()
    );

    let cells = file.entries.len() * Rank::DIVISIONS.len();
    let present: usize = file
        .entries
        .iter()
        .map(|entry| {
            Rank::DIVISIONS
                .iter()
                .filter(|rank| entry.value_for(**rank).is_some())
                .count()
        })
        .sum();
    assert!(
        present * 10 >= cells * 9,
        "only {present} of {cells} rank cells are filled"
    );

    // The assertion that matters. A file where every hero reads the same at
    // every rung is a rank picker wired to nothing.
    let varying = file
        .entries
        .iter()
        .filter(|entry| {
            let mut columns = Rank::DIVISIONS.iter().map(|rank| entry.value_for(*rank));
            let first = columns.next().flatten();
            columns.any(|value| value != first)
        })
        .count();
    assert!(
        varying * 2 >= ds.hero_count(),
        "only {varying} of {} heroes move across the ladder at all - \
         the rank columns are probably all one tier's numbers",
        ds.hero_count()
    );
}

/// A rank axis stored backwards passes every count and every coverage check, and
/// would quietly hand every player the advice for the far end of the ladder.
/// These are the two largest measured swings on the committed data, pinned by
/// direction and a wide margin rather than by magnitude.
///
/// Like the named matchup pins above, these are claims about live heroes and a
/// balance patch may well come for them. A failure here is a prompt to go and
/// look, not necessarily a bug.
#[test]
fn the_low_rank_heroes_and_the_high_rank_heroes_land_on_the_right_ends_of_the_ladder() {
    let ds = load().expect("committed data must load");

    // Reinhardt is the archetypal low-rank tank: he shepherds a Bronze lobby and
    // gets walked around by a coordinated one.
    let reinhardt = ds.hero_by_key("reinhardt").expect("on the roster");
    assert!(
        ds.rank_shift(Rank::Bronze, reinhardt) > ds.rank_shift(Rank::Grandmaster, reinhardt) + 30,
        "Reinhardt's rank curve has lost its direction: bronze {} vs grandmaster {}",
        ds.rank_shift(Rank::Bronze, reinhardt),
        ds.rank_shift(Rank::Grandmaster, reinhardt)
    );

    // Ashe is the mirror: a hitscan whose value is all in whether the shots land.
    let ashe = ds.hero_by_key("ashe").expect("on the roster");
    assert!(
        ds.rank_shift(Rank::Grandmaster, ashe) > ds.rank_shift(Rank::Bronze, ashe) + 30,
        "Ashe's rank curve has lost its direction: bronze {} vs grandmaster {}",
        ds.rank_shift(Rank::Bronze, ashe),
        ds.rank_shift(Rank::Grandmaster, ashe)
    );
}

/// The aggregate is not a rung, and reading it must be exactly what this app did
/// before the picker existed. If this ever fails, everybody who never opens the
/// control has silently been given somebody else's answer.
#[test]
fn the_whole_ladder_reads_as_the_strength_file_it_has_always_read() {
    let ds = load().expect("committed data must load");

    for index in 0..ds.hero_count() {
        let hero = HeroId(index as u16);
        assert_eq!(ds.rank_shift(Rank::All, hero), 0);
        assert_eq!(ds.base_strength_at(Rank::All, hero), ds.base_strength(hero));
    }
}

/// The guard that stands in for widening the win-rate band.
///
/// `WIN_RATE_FLOOR`/`WIN_RATE_CEILING` assert that ±6 points is the whole
/// meaningful range of a hero win rate, and the rank slices are the first thing
/// that pushes on it — a hero already at the top of the band cannot be reported
/// as better still at the rung that suits it. A handful of saturated cells is the
/// scale honestly saying "as far as it goes"; a lot of them means the band has
/// stopped describing the data, and the band is then the thing to argue about
/// rather than this test.
#[test]
fn almost_none_of_the_rank_shifts_saturate_the_scale() {
    let ds = load().expect("committed data must load");

    let saturated = (0..ds.hero_count())
        .flat_map(|index| Rank::DIVISIONS.map(|rank| ds.rank_shift(rank, HeroId(index as u16))))
        .filter(|value| value.abs() == 100)
        .count();
    let cells = ds.hero_count() * Rank::DIVISIONS.len();

    assert!(
        saturated * 10 <= cells,
        "{saturated} of {cells} rank cells are pinned at the rails - \
         the 44..56 win-rate band no longer describes the data"
    );
}

/// The one place the rank picker changes a number a reader can check: the ban
/// list's patch rung. It has to move, or the picker is decorative there.
#[test]
fn the_ban_lists_patch_rung_moves_with_the_rank() {
    let ds = load().expect("committed data must load");
    let draft = Draft::new();
    let team = DefendedTeam::default();

    let at = |rank: Rank| {
        let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
        ctx.rank = rank;
        let board = ban_recommendations(&ds, &draft, &ctx, &team);
        assert_eq!(board.subject, BanSubject::Patch);
        board
            .candidates
            .iter()
            .take(5)
            .map(|c| c.hero)
            .collect::<Vec<_>>()
    };

    assert_ne!(
        at(Rank::Bronze),
        at(Rank::Grandmaster),
        "the same five heroes lead the patch rung at both ends of the ladder"
    );
}

/// The guard on the top of the matchup scale, and the counterpart to
/// `almost_none_of_the_rank_shifts_saturate_the_scale`.
///
/// `-100..=100` claims to describe every matchup in the game, so the rail has to
/// mean "as lopsided as this gets" rather than "somewhat lopsided". A handful of
/// rows up there is the scale working; a lot of them means the extreme end has
/// stopped separating a hard counter from a losing duel, and the conversion is
/// then the thing to argue about rather than this test.
///
/// Counted over rated rows rather than over all `n x n` cells, because an unrated
/// pair is not a reading at the middle of the scale — it is the absence of one.
#[test]
fn the_extreme_end_of_the_matchup_scale_stays_rare() {
    let matchups: MatchupsFile =
        toml::from_str(overwatch_data::MATCHUPS_TOML).expect("committed matchups must parse");

    let rated = matchups.matchups.len();
    // `resolved()` and not `value`, because `value` is only ever the blend: a
    // hand-written `curated = 95` would otherwise sail straight past the one
    // guard on the top of the scale, and the curated column is precisely the one
    // a person can set without a source arguing them down.
    let extreme = matchups
        .matchups
        .iter()
        .filter(|entry| entry.resolved().abs() >= 90)
        .count();

    // 22 of 2535 today, a little under 1%.
    assert!(
        extreme * 50 <= rated,
        "{extreme} of {rated} rated rows sit at |resolved| >= 90, which is more of the \
         matchup scale's top end than a healthy blend reaches"
    );
}

/// Nobody wins a matchup from both sides of it.
///
/// The matrix is deliberately not forced to be antisymmetric — see `Matrix` — so
/// nothing in the pipeline makes this true by construction, and until the blend
/// reached its verdicts per pair rather than per row, the mirror of a corrected
/// row went uncorrected. This is the invariant that failure violated in spirit
/// even where it did not violate it in arithmetic.
///
/// An empirical guard, then: it can only fire if the secondary source moves one
/// direction of a pair past the primary's antisymmetry, and if it ever does, the
/// thing to argue about is the blend and not this assertion.
#[test]
fn no_pair_reads_as_favourable_for_both_heroes() {
    let ds = load().expect("committed data must load");

    let n = ds.hero_count();
    let mut both = Vec::new();
    for a in 0..n {
        for b in (a + 1)..n {
            let (a, b) = (HeroId(a as u16), HeroId(b as u16));
            if let (Some(forward), Some(reverse)) =
                (ds.matchups().rating(a, b), ds.matchups().rating(b, a))
            {
                if forward > 0 && reverse > 0 {
                    both.push((ds.hero(a).map(|h| h.key.clone()), forward, reverse));
                }
            }
        }
    }

    assert!(
        both.is_empty(),
        "{} pair(s) read as favourable in both directions: {both:?}",
        both.len()
    );
}

/// A curated matchup is curated from both sides, or from neither.
///
/// The scorer folds a pair as `(forward - reverse) / 2` whenever *both*
/// directions are rated — and a measured zero is rated, which is the state most
/// of the pairs worth curating are in. So a curated `+40` opposite a scraped `0`
/// reaches the score as `+20`, and the reason line then argues half of what the
/// note says. That is not a wrong number the review would catch; it is the right
/// number quietly halved.
///
/// The mirror may also be *unrated*, which is the one honest one-sided case:
/// `matchup_term` uses a lone reading at full magnitude rather than folding the
/// absent direction in as a zero.
///
/// Reads the TOML directly because the loader collapses `curated` into the matrix
/// through `resolved()` and cannot tell an override from a blend afterwards.
#[test]
fn a_curated_matchup_is_curated_from_both_sides() {
    let matchups: MatchupsFile =
        toml::from_str(overwatch_data::MATCHUPS_TOML).expect("committed matchups must parse");

    let rated: HashSet<(&str, &str)> = matchups
        .matchups
        .iter()
        .map(|entry| (entry.hero.as_str(), entry.vs.as_str()))
        .collect();
    let curated: HashSet<(&str, &str)> = matchups
        .matchups
        .iter()
        .filter(|entry| entry.curated.is_some())
        .map(|entry| (entry.hero.as_str(), entry.vs.as_str()))
        .collect();

    let halved: Vec<(&str, &str)> = curated
        .iter()
        .copied()
        .filter(|(hero, vs)| {
            let mirror = (*vs, *hero);
            rated.contains(&mirror) && !curated.contains(&mirror)
        })
        .collect();

    assert!(
        halved.is_empty(),
        "{} curated row(s) have a rated mirror that is not curated, so the pair scores at \
         half the curated magnitude: {halved:?}",
        halved.len()
    );

    // The other half of the rule, and the reason it is not merely "curate both":
    // two positives read as both heroes winning, which
    // `no_pair_reads_as_favourable_for_both_heroes` rejects outright.
    let notes = matchups
        .matchups
        .iter()
        .filter(|entry| entry.curated.is_some() && entry.note.trim().is_empty())
        .count();
    assert_eq!(
        notes, 0,
        "{notes} curated row(s) carry no note - a number with no source behind it is \
         indistinguishable from a typo unless it says why"
    );
}

/// How much of the support mirror the data has an opinion about.
///
/// The tool was reviewed in public and told, correctly, that it never suggests
/// banning a support. The cause is not the scorer: duel-derived sources cannot see
/// Suzu cleansing a nade or a lamp eating a burst, so they read those pairs as
/// dead even, and an even pair argues for nothing. This is the measurement of that
/// gap, so that closing it is visible and reopening it is not silent.
///
/// The bound is a ceiling on *dead-even* readings rather than a floor on threats,
/// because a support genuinely can be favoured into most of the mirror — what
/// cannot be true is that the whole role is a coin flip against itself.
///
/// Folds the pair the way `matchup_term` does rather than reading one row, because
/// the folded reading is what the scorer sees. `matchup_term` is private to
/// `overwatch-core`, so the four arms are repeated here.
#[test]
fn the_support_mirror_is_not_mostly_dead_even() {
    let ds = load().expect("committed data must load");

    let supports: Vec<HeroId> = ds.heroes_in_role(Role::Support).collect();
    let mut worst = 0;
    let mut table: Vec<String> = Vec::new();

    for hero in &supports {
        let (mut threat, mut even, mut favoured, mut unrated) = (0, 0, 0, 0);
        for enemy in &supports {
            if hero == enemy {
                continue;
            }
            let forward = ds.matchups().rating(*hero, *enemy).map(f32::from);
            let reverse = ds.matchups().rating(*enemy, *hero).map(f32::from);
            match (forward, reverse) {
                (Some(f), Some(r)) => {
                    let term = (f - r) / 2.0;
                    if term < 0.0 {
                        threat += 1;
                    } else if term == 0.0 {
                        even += 1;
                    } else {
                        favoured += 1;
                    }
                }
                (Some(f), None) if f < 0.0 => threat += 1,
                (None, Some(r)) if r > 0.0 => threat += 1,
                (Some(0.0), None) | (None, Some(0.0)) => even += 1,
                (Some(_), None) | (None, Some(_)) => favoured += 1,
                (None, None) => unrated += 1,
            }
        }

        worst = worst.max(even);
        let name = ds
            .hero(*hero)
            .map(|h| h.key.as_str())
            .unwrap_or("?")
            .to_owned();
        table.push(format!(
            "{name}: {threat} lose, {even} even, {favoured} win, {unrated} unrated"
        ));
    }

    // 8 today, both Kiriko and Lifeweaver, out of 13 opponents each. Every value
    // curated brings this down; tighten the bound when it does, because a bound
    // that never moves is a bound nobody is reading.
    assert!(
        worst <= 8,
        "a support reads dead even against more than 8 of the other 13 supports, \
         which is more of the role than the counter data can honestly call a coin flip\n  {}",
        table.join("\n  ")
    );
}

/// The guard on the counterwatch column, and the counterpart to
/// `almost_none_of_the_rank_shifts_saturate_the_scale`.
///
/// `COUNTER_CEILING` asserts that a published counter rating of 25 is as lopsided
/// as that source gets, and it is the one scale in the pipeline set from a
/// distribution rather than from a source's own documented range. A handful of
/// readings at the rail is the ceiling honestly saying "as far as it goes"; a lot
/// of them means the band has stopped describing what the site publishes, and the
/// band is then the thing to argue about rather than this test.
///
/// Counted over readings and not over rows, because most of the matrix has no
/// counterwatch opinion at all and folding that in would hide a saturating column
/// behind the size of the file.
#[test]
fn almost_none_of_the_counter_readings_saturate_the_scale() {
    let matchups: MatchupsFile =
        toml::from_str(overwatch_data::MATCHUPS_TOML).expect("committed matchups must parse");

    let readings: Vec<i8> = matchups
        .matchups
        .iter()
        .filter_map(|entry| entry.cwatch)
        .collect();
    let saturated = readings.iter().filter(|value| value.abs() == 100).count();

    assert!(
        !readings.is_empty(),
        "counterwatch has no readings in the committed matrix at all"
    );
    // 4 of 795 today, all of them D.Mon, whose ratings are the two outliers the
    // site publishes.
    assert!(
        saturated * 50 <= readings.len(),
        "{saturated} of {} counterwatch readings are pinned at the rails - the \
         +-25 rating band no longer describes what the site publishes",
        readings.len()
    );
}

/// The guard on the newest generated file, in the shape
/// `the_rank_shifts_are_populated_and_actually_differ_from_one_another` uses.
///
/// Coverage and **variation**, because coverage alone cannot tell nine readings
/// from one reading written nine times — and that is what every realistic way of
/// getting this wrong produces. The cache key without its tier, a rung the source
/// stopped publishing, a nine-wide table read at `Rank::column` instead of
/// `Rank::slot`: each one yields a file that parses, loads, scores and satisfies
/// any count of filled cells.
///
/// It also pins the zero point, which is the part with no data in it: summed over
/// a role, pick rate comes to exactly `100 x slots(role)`, so the role mean *is*
/// the fair share and the values in a role must therefore straddle zero. A column
/// where every hero reads positive is a column measured against the wrong
/// denominator.
#[test]
fn the_prevalence_columns_are_populated_and_actually_differ_from_one_another() {
    let ds = load().expect("committed data must load");
    let n = ds.hero_count();

    let mut rated = 0;
    let mut varying = 0;
    for index in 0..n {
        let hero = HeroId(index as u16);
        let readings: Vec<i8> = Rank::CHOICES
            .iter()
            .map(|rank| ds.prevalence_at(*rank, hero))
            .collect();

        if readings.iter().any(|value| *value != 0) {
            rated += 1;
        }
        if readings.iter().any(|value| *value != readings[0]) {
            varying += 1;
        }
    }

    assert!(
        rated * 10 >= n * 8,
        "only {rated}/{n} heroes have a prevalence reading anywhere"
    );
    assert!(
        varying * 2 >= n,
        "{varying}/{n} heroes read differently across the rungs - nine identical \
         columns is what a cache key without its tier produces"
    );

    // Nobody is above their share at every rung and nobody below it at every one,
    // because the share is the role's own mean.
    for role in Role::ALL {
        let heroes: Vec<HeroId> = ds.heroes_in_role(role).collect();
        let all_ranks: Vec<i8> = heroes
            .iter()
            .map(|hero| ds.prevalence_at(Rank::All, *hero))
            .collect();
        assert!(
            all_ranks.iter().any(|value| *value > 0) && all_ranks.iter().any(|value| *value < 0),
            "{role:?} prevalence has to straddle zero, because zero is that role's \
             own mean by construction"
        );
    }
}

/// The failure the selection shrink could cause rather than fix.
///
/// It pulls a hero's win rate toward its role's mean in proportion to how rarely
/// it is picked, which is the correction a specialist-only win rate needs. Applied
/// too hard, though, it would turn `strength.toml` into a ranking of pick rate:
/// every rarely-picked hero flattened to average and every common one left alone,
/// so the strongest heroes would simply be the most-played ones — or, if the sign
/// went wrong, the least.
///
/// So this asserts the two columns are not measuring the same thing. Deliberately
/// a weak bound in both directions: some genuine relationship is expected and
/// fine, because a hero that is actually strong does get picked more.
#[test]
fn the_strongest_heroes_are_not_simply_the_least_picked_ones() {
    let ds = load().expect("committed data must load");

    let pairs: Vec<(f32, f32)> = (0..ds.hero_count())
        .map(|index| HeroId(index as u16))
        .map(|hero| {
            (
                f32::from(ds.base_strength(hero)),
                f32::from(ds.prevalence_at(Rank::All, hero)),
            )
        })
        .collect();

    let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
    let strengths: Vec<f32> = pairs.iter().map(|(s, _)| *s).collect();
    let picks: Vec<f32> = pairs.iter().map(|(_, p)| *p).collect();
    let (mean_s, mean_p) = (mean(&strengths), mean(&picks));

    let covariance: f32 = pairs
        .iter()
        .map(|(s, p)| (s - mean_s) * (p - mean_p))
        .sum::<f32>();
    let spread = |values: &[f32], mean: f32| {
        values
            .iter()
            .map(|v| (v - mean).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    let r = covariance / (spread(&strengths, mean_s) * spread(&picks, mean_p));

    assert!(
        r.abs() < 0.7,
        "base strength and prevalence correlate at r = {r:.2} - patch strength has \
         become a reading of how often a hero is picked rather than of how often \
         it wins"
    );
}

/// `data/ban_rate.toml` is the one file in `data/` the app must never load.
///
/// It measures who gets banned rather than who is strong — a second argument the
/// ban list refuses to add to the first — and it is the quantity the acceptance
/// test below *predicts*, so scoring on it would make that test circular. The only
/// thing standing between "a yardstick" and "a term somebody wired up" is an
/// `include_str!` that does not exist, which is exactly the kind of absence that
/// gets added back by accident.
///
/// So this reads the loader's own source and asserts the file is not in it. The
/// test file reaches the data directly instead, which is what keeps the table out
/// of a 1.2 MB wasm bundle it has no business being in.
#[test]
fn the_ban_rate_table_never_reaches_the_bundle() {
    const LOADER: &str = include_str!("../src/lib.rs");

    assert!(
        !LOADER.contains("ban_rate"),
        "overwatch-data/src/lib.rs mentions ban_rate, which means the yardstick is \
         being compiled into the app it is supposed to be judging"
    );
    // And the guard is only worth anything while the file it guards exists.
    assert!(
        !BAN_RATE_TOML.is_empty(),
        "data/ban_rate.toml is empty - run `just ingest-strength`"
    );
}

/// The acceptance test for the whole prevalence feature: does the ban list move
/// toward what Grandmaster players actually ban?
///
/// **Grandmaster only, on purpose.** rho(ban rate, pick rate) by rung runs 0.04 at
/// Bronze and Gold, 0.17 at Diamond, 0.36 at Master and 0.52 at Grandmaster. Below
/// Diamond the ban button is spent on annoyance rather than on strength — Sombra
/// is banned in 73.2% of Bronze games and 0.1% of Grandmaster ones — so the
/// ladder's ban rate down there is not ground truth for anything this app computes.
///
/// Three assertions, in descending order of how stable they should be. What it
/// deliberately does **not** assert is worth writing down too, because both look
/// like obvious additions:
///
/// - Not "the top eight are heroes the ladder bans". The median Grandmaster ban
///   rate is 1.90%: the ladder spends its bans on two or three broken heroes, and a
///   ranked eight cannot look like that without being wrong about the other five.
/// - Not "Torbjörn is out of the top eight". That is a one-place margin a balance
///   patch owns, and it is not this term's job anyway — reading Blizzard's win rate
///   and correcting for selection are what moved him.
#[test]
fn the_patch_rung_at_grandmaster_moves_toward_what_grandmasters_actually_ban() {
    let ds = load().expect("committed data must load");
    let bans: BanRateFile = toml::from_str(BAN_RATE_TOML).expect("the yardstick must parse");
    let published: std::collections::HashMap<&str, f32> = bans
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .value_for(Rank::Grandmaster)
                .map(|rate| (entry.hero.as_str(), rate))
        })
        .collect();

    let board = |weight: f32| {
        let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
        ctx.rank = Rank::Grandmaster;
        ctx.weights.prevalence = weight;
        let board = ban_recommendations(&ds, &Draft::new(), &ctx, &DefendedTeam::default());
        assert_eq!(board.subject, BanSubject::Patch);
        board
            .candidates
            .iter()
            .map(|candidate| {
                let key = ds.hero(candidate.hero).expect("in range").key.clone();
                let rate = published.get(key.as_str()).copied().unwrap_or(0.0);
                (candidate.score, rate)
            })
            .collect::<Vec<_>>()
    };

    let discounted = board(0.40);
    let plain = board(0.0);

    // 1. It agrees with the ladder better than it did. Positive and strictly
    //    improved, which is the claim; the magnitudes move with every patch.
    let rho = |rows: &[(f32, f32)]| {
        let rank_of = |values: Vec<f32>| {
            let mut order: Vec<usize> = (0..values.len()).collect();
            order.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
            let mut ranks = vec![0.0f32; values.len()];
            for (position, index) in order.into_iter().enumerate() {
                ranks[index] = position as f32;
            }
            ranks
        };
        let xs = rank_of(rows.iter().map(|(score, _)| *score).collect());
        let ys = rank_of(rows.iter().map(|(_, rate)| *rate).collect());
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let (mx, my) = (mean(&xs), mean(&ys));
        let cov: f32 = xs.iter().zip(&ys).map(|(a, b)| (a - mx) * (b - my)).sum();
        let dev = |v: &[f32], m: f32| v.iter().map(|x| (x - m).powi(2)).sum::<f32>().sqrt();
        cov / (dev(&xs, mx) * dev(&ys, my))
    };
    let (with, without) = (rho(&discounted), rho(&plain));
    assert!(
        with > 0.0 && with > without,
        "the discount has to improve agreement with the ladder, not just change it: \
         rho {without:.3} without it, {with:.3} with"
    );

    // 2. The heroes the ladder bans hardest are on the list somewhere. Three of
    //    five rather than five of five, because two of them are heroes our sources
    //    say beat nobody, and this list only carries heroes there is an argument
    //    for.
    let mut ctx = UserContext::new(Role::Tank, ds.hero_count());
    ctx.rank = Rank::Grandmaster;
    let candidates = ban_recommendations(&ds, &Draft::new(), &ctx, &DefendedTeam::default());
    let on_the_list: std::collections::HashSet<&str> = candidates
        .candidates
        .iter()
        .filter_map(|candidate| ds.hero(candidate.hero).ok().map(|hero| hero.key.as_str()))
        .collect();

    let mut hardest: Vec<(&str, f32)> = published.iter().map(|(k, v)| (*k, *v)).collect();
    hardest.sort_by(|a, b| b.1.total_cmp(&a.1));
    let listed = hardest
        .iter()
        .take(5)
        .filter(|(hero, _)| on_the_list.contains(hero))
        .count();
    assert!(
        listed >= 3,
        "only {listed} of the five most-banned heroes at grandmaster appear on the \
         list at all"
    );

    // 3. And the top of the list is dangerous by the ladder's own reckoning, not
    //    merely sorted. Against a roster mean of 7.35%.
    let top_eight = discounted.iter().take(8).map(|(_, rate)| rate).sum::<f32>() / 8.0;
    let roster = published.values().sum::<f32>() / published.len() as f32;
    assert!(
        top_eight >= roster * 2.0,
        "the top eight average a {top_eight:.1}% ban rate against a roster mean of \
         {roster:.1}% - the list is not finding the heroes the ladder fears"
    );
}
