//! The part of the state that outlives a match, persisted to `localStorage`.
//!
//! Re-entering your hero pool at the start of every session would cost more
//! time than the app saves, so pool, role, weights and personal overrides are
//! sticky. The map is sticky too, but only within a session — it is cleared
//! deliberately rather than carried into the next match on a different map.

use overwatch_core::{Dataset, Format, HeroSet, Rank, Role, UserContext, Weights};
use serde::{Deserialize, Serialize};

const STORAGE_KEY: &str = "minmax.profile";

/// The key this was stored under before the app was named. Read once, migrated
/// forward, then removed — a rename that silently emptied everyone's hero pool
/// would cost exactly the setup time the profile exists to save.
const LEGACY_STORAGE_KEY: &str = "overwatch-picker.profile";

/// Stored by hero *key* rather than index, so a roster update — which shifts
/// every index — cannot silently repoint your pool at the wrong heroes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredProfile {
    #[serde(default)]
    pub role: Option<String>,
    // One field per role rather than a map keyed by role name: the names are
    // already in the stored form of every profile written so far, and renaming
    // them would silently cost people the pools they have.
    #[serde(default)]
    pub tank_pool: Vec<String>,
    /// Added with the damage pick mode. `default` rather than required, so a
    /// profile written before it existed loads with its other two pools intact
    /// and simply starts damage empty.
    #[serde(default)]
    pub damage_pool: Vec<String>,
    #[serde(default)]
    pub support_pool: Vec<String>,
    /// The queue you were last in. Sticky like the role and unlike the map: you
    /// play an evening of 6v6, while the map changes every game.
    ///
    /// Stored as the typed value rather than a pair of strings, the way
    /// `weights` is — the variant names *are* the stable keys, and core pins
    /// them.
    #[serde(default)]
    pub format: Option<Format>,
    /// The rung of the ladder you last read patch strength on.
    ///
    /// A key rather than the typed value, unlike `format` above, and the reason
    /// is that the divisions are not a closed set the way `TeamSize` and `Queue`
    /// are: Blizzard inserted Emerald between Platinum and Diamond and added
    /// Champion above Grandmaster. An unknown enum variant is a hard serde error
    /// on the whole struct, and `Profile::load` swallows that into a default —
    /// so a rung a newer build wrote would cost an older one (a second machine,
    /// a stale service worker) that person's entire pool, weights and session.
    /// A string parsed with `Rank::parse` costs them the setting and nothing
    /// else.
    #[serde(default)]
    pub rank: Option<String>,
    /// Comfort adjustments, keyed by hero.
    #[serde(default)]
    pub overrides: Vec<(String, i8)>,
    #[serde(default)]
    pub weights: Option<Weights>,
    /// What the session roster calls you.
    #[serde(default)]
    pub name: Option<String>,
    /// The session to rejoin on the next visit, so a team that drafts all
    /// evening types the code once.
    #[serde(default)]
    pub session: Option<String>,
    /// The room name from before sessions existed.
    ///
    /// Read as a fallback for `session` and never written again. A profile
    /// written by the two-person build is by definition older than this code,
    /// and dropping the field would silently move someone out of the room they
    /// had been using into no session at all.
    #[serde(default)]
    pub room: Option<String>,
}

/// The resolved, in-memory form.
#[derive(Debug, Clone)]
pub struct Profile {
    pub role: Role,
    /// The format a fresh, solo board opens in. In a session the room's board
    /// wins, exactly as the map does — this only ever remembers what *you* last
    /// chose, and an incoming board never writes it.
    pub format: Format,
    /// The heroes you actually play, one set per role, indexed by
    /// [`Role::index`]. An array rather than a field each, so adding a fourth
    /// role could never again leave a `match` with a catch-all arm quietly
    /// pointing two roles at one pool.
    ///
    /// Purely a marker: it highlights your picks in the list rather than
    /// deciding what appears there. It used to be a whitelist, and a separate
    /// "favourites" set did the highlighting — two levers for one idea, and the
    /// filtering one hid heroes on exactly the draft that called for them. The
    /// comfort overrides remain the lever for "rank this hero higher".
    pub pools: [HeroSet; Role::ALL.len()],
    /// Which rung of the ladder to read patch strength on. Sticky like the role
    /// and the format — you play an evening in one bracket.
    pub rank: Rank,
    pub overrides: Vec<i8>,
    pub weights: Weights,
    /// What the roster calls you. Empty until you say otherwise, at which point
    /// the seat falls back to showing your client id.
    pub name: String,
    /// The last session joined, if any. `None` is drafting alone, which is a
    /// perfectly ordinary way to use the app rather than a missing setting.
    pub session: Option<String>,
}

impl Profile {
    pub fn empty(hero_count: usize) -> Self {
        Self {
            role: Role::Tank,
            format: Format::default(),
            pools: [HeroSet::empty(); Role::ALL.len()],
            rank: Rank::All,
            overrides: vec![0; hero_count],
            weights: Weights::default(),
            name: String::new(),
            session: None,
        }
    }

    /// The scoring context this profile describes.
    ///
    /// Assembled here rather than at the call sites because there are two of
    /// them — the pick column and `frame_top`, the hero the lock-top shortcut
    /// takes — and a context that differs between them locks a hero the list
    /// never showed at the top. Every field the scorer reads off a profile is
    /// gathered in one place so a new one cannot reach only half the screen.
    ///
    /// After this existed, `UserContext::new` has no other caller in this crate.
    /// That is the invariant, not a coincidence.
    pub fn context(&self, hero_count: usize) -> UserContext {
        let mut ctx = UserContext::new(self.role, hero_count);
        ctx.rank = self.rank;
        ctx.overrides = self.overrides.clone();
        ctx.weights = self.weights;
        ctx
    }

    pub fn pool(&self, role: Role) -> HeroSet {
        self.pools[role.index()]
    }

    pub fn pool_mut(&mut self, role: Role) -> &mut HeroSet {
        &mut self.pools[role.index()]
    }

    fn from_stored(stored: StoredProfile, dataset: &Dataset) -> Self {
        let mut profile = Self::empty(dataset.hero_count());

        if let Some(role) = stored.role.as_deref().and_then(|r| Role::parse(r).ok()) {
            profile.role = role;
        }
        // A profile written before formats existed starts you where it left
        // everyone: 5v5 role queue, which is the only shape the app had.
        if let Some(format) = stored.format {
            profile.format = format;
        }
        // A profile written before ranks existed, or one naming a division this
        // build has never heard of, reads as `Rank::All` — the whole ladder,
        // which is what this app scored on before the picker existed. So it
        // opens exactly where it left you, and a rung from the future costs the
        // setting rather than the profile.
        if let Some(rank) = stored.rank.as_deref().and_then(|r| Rank::parse(r).ok()) {
            profile.rank = rank;
        }
        // Unknown keys are dropped rather than treated as an error: a hero
        // removed from the roster should cost you that entry, not your profile.
        *profile.pool_mut(Role::Tank) = to_set(&stored.tank_pool, dataset);
        *profile.pool_mut(Role::Damage) = to_set(&stored.damage_pool, dataset);
        *profile.pool_mut(Role::Support) = to_set(&stored.support_pool, dataset);
        // A profile written before the pool absorbed the favourites still
        // carries a `favorites` key. Unknown fields are ignored rather than
        // rejected, so it loads with its pools intact and the stale key goes
        // away on the next save.

        for (key, value) in &stored.overrides {
            if let Ok(hero) = dataset.hero_by_key(key) {
                if let Some(slot) = profile.overrides.get_mut(hero.index()) {
                    *slot = *value;
                }
            }
        }
        if let Some(weights) = stored.weights {
            profile.weights = weights;
        }
        if let Some(name) = stored.name {
            profile.name = name;
        }
        // `room` is what this field was called before sessions, and a profile
        // still carrying one should land back in it rather than nowhere.
        // Deliberately *not* falling back to `room`. A room was created by the
        // first person to join it, so the old default "us" meant something; a
        // session has to be minted, so the same string now names a session that
        // by definition does not exist. Carrying it forward auto-joined every
        // upgraded client into a session the server could only reject, and left
        // the bar showing that dead code instead of a way to start a real one.
        // Starting alone is the honest reading of a profile written before
        // sessions existed.
        profile.session = stored.session.filter(|code| !code.trim().is_empty());
        profile
    }

    fn to_stored(&self, dataset: &Dataset) -> StoredProfile {
        let keys = |set: &HeroSet| {
            set.iter()
                .filter_map(|hero| dataset.hero(hero).ok())
                .map(|hero| hero.key.clone())
                .collect()
        };

        StoredProfile {
            role: Some(self.role.as_str().to_owned()),
            format: Some(self.format),
            rank: Some(self.rank.as_str().to_owned()),
            tank_pool: keys(&self.pool(Role::Tank)),
            damage_pool: keys(&self.pool(Role::Damage)),
            support_pool: keys(&self.pool(Role::Support)),
            overrides: self
                .overrides
                .iter()
                .enumerate()
                .filter(|(_, value)| **value != 0)
                .filter_map(|(i, value)| {
                    dataset
                        .heroes()
                        .get(i)
                        .map(|hero| (hero.key.clone(), *value))
                })
                .collect(),
            weights: Some(self.weights),
            name: Some(self.name.clone()),
            session: self.session.clone(),
            // Deliberately not written back. Keeping it would mean two fields
            // claiming to say which session you are in, and the older one
            // winning on the next machine that reads the profile.
            room: None,
        }
    }

    /// Reads the stored profile, falling back to defaults on anything corrupt.
    /// A bad profile must never stop the app from opening mid-draft.
    pub fn load(dataset: &Dataset) -> Self {
        let stored = storage()
            .and_then(|s| read_raw(&s))
            .and_then(|raw| serde_json::from_str::<StoredProfile>(&raw).ok())
            .unwrap_or_default();
        Self::from_stored(stored, dataset)
    }

    /// Best-effort save; persistence failing is not worth interrupting a draft.
    pub fn save(&self, dataset: &Dataset) {
        let Ok(raw) = serde_json::to_string(&self.to_stored(dataset)) else {
            return;
        };
        if let Some(storage) = storage() {
            let _ = storage.set_item(STORAGE_KEY, &raw);
        }
    }
}

fn to_set(keys: &[String], dataset: &Dataset) -> HeroSet {
    let mut set = HeroSet::empty();
    for key in keys {
        if let Ok(hero) = dataset.hero_by_key(key) {
            let _ = set.insert(hero);
        }
    }
    set
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// The stored profile, migrating it off the pre-rename key if that is where it
/// still lives.
///
/// The write is what makes this a migration rather than a permanent fallback,
/// and the remove is what stops a stale copy from winning later: without it, a
/// profile saved here and then read on a build that still preferred the old key
/// would silently roll back. Both are best-effort — a storage that refuses the
/// write still returns the profile, it just migrates again next time.
fn read_raw(storage: &web_sys::Storage) -> Option<String> {
    if let Ok(Some(raw)) = storage.get_item(STORAGE_KEY) {
        return Some(raw);
    }
    let raw = storage.get_item(LEGACY_STORAGE_KEY).ok().flatten()?;
    let _ = storage.set_item(STORAGE_KEY, &raw);
    let _ = storage.remove_item(LEGACY_STORAGE_KEY);
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use overwatch_core::{Queue, TeamSize};

    /// A stored profile is by definition older than the code reading it, and
    /// [`Profile::load`] falls back to defaults on anything it cannot parse —
    /// so a field this build has dropped must be ignored rather than rejected.
    /// `focus_multiplier` went with the "playing well" flag; a profile still
    /// carrying it has to load with its pools and its other weights intact,
    /// because the alternative is silently emptying someone's pool.
    #[test]
    fn a_profile_written_before_the_focus_flag_was_removed_still_loads() {
        let old = r#"{
            "role": "tank",
            "tank_pool": ["reinhardt", "winston"],
            "damage_pool": [],
            "support_pool": ["ana"],
            "overrides": [["dva", 20]],
            "weights": {
                "base": 0.15,
                "counter": 1.0,
                "synergy": 0.3,
                "map": 0.25,
                "personal": 0.6,
                "side": 0.2,
                "focus_multiplier": 1.8,
                "swap_threshold": 0.15
            },
            "room": "us"
        }"#;

        let stored: StoredProfile =
            serde_json::from_str(old).expect("an older profile still parses");

        assert_eq!(stored.tank_pool, vec!["reinhardt", "winston"]);
        assert_eq!(stored.support_pool, vec!["ana"]);
        assert_eq!(stored.overrides, vec![("dva".to_owned(), 20)]);
        let weights = stored
            .weights
            .expect("the weights survive the dropped field");
        assert_eq!(weights.personal, 0.6);
        assert_eq!(weights.swap_threshold, 0.15);
        // The other half of the same rule, and the more dangerous half: a weight
        // this build has *added* has to arrive at its real default rather than
        // at serde's 0.0 for `f32`. This blob predates the rank term entirely,
        // and a 0.0 here would ship the rank picker wired to nothing for
        // everybody who already has a profile — which is everybody who has used
        // the app. See `default_rank` in core.
        assert_eq!(weights.rank, Weights::default().rank);
        assert!(weights.rank > 0.0);
        assert_eq!(weights.shape, Weights::default().shape);
        // And the same again for the prevalence discount, where the failure looks
        // harmless instead of obvious: it is a *multiplier*, so serde's 0.0 does
        // not zero a term — it makes the factor exactly 1.0, which is the ban list
        // ignoring pick rate entirely. See `default_prevalence` in core.
        assert_eq!(weights.prevalence, Weights::default().prevalence);
        assert!(weights.prevalence > 0.0);
    }

    /// The two-person build called this field `room` and defaulted it to "us",
    /// so essentially every pre-session profile carries one. A room was created
    /// by whoever joined it first; a session has to be minted. Carrying the name
    /// across therefore auto-joined every upgraded client into a session the
    /// server could only reject — and the bar then showed that dead code where
    /// "start a session" belonged. Starting alone is the honest reading.
    #[test]
    fn a_profile_that_still_says_room_starts_you_alone() {
        let stored = StoredProfile {
            room: Some("us".to_owned()),
            ..StoredProfile::default()
        };

        let profile = Profile::from_stored(stored, &fixture());
        assert_eq!(
            profile.session, None,
            "an old room name is not a session anybody minted"
        );
    }

    #[test]
    fn a_real_session_survives_a_stale_room_beside_it() {
        let stored = StoredProfile {
            session: Some("brave-otter-41".to_owned()),
            room: Some("us".to_owned()),
            ..StoredProfile::default()
        };

        let profile = Profile::from_stored(stored, &fixture());
        assert_eq!(profile.session.as_deref(), Some("brave-otter-41"));
    }

    #[test]
    fn a_profile_that_names_no_session_leaves_you_drafting_alone() {
        let profile = Profile::from_stored(StoredProfile::default(), &fixture());
        assert_eq!(profile.session, None);
        assert_eq!(profile.name, "");
    }

    /// A saved profile must not resurrect the old field, or the next load would
    /// have two answers to "which session" and pick the stale one.
    #[test]
    fn saving_stops_writing_the_field_sessions_replaced() {
        let dataset = fixture();
        let mut profile = Profile::empty(dataset.hero_count());
        profile.session = Some("brave-otter-41".to_owned());
        profile.name = "era".to_owned();

        let stored = profile.to_stored(&dataset);
        assert_eq!(stored.session.as_deref(), Some("brave-otter-41"));
        assert_eq!(stored.name.as_deref(), Some("era"));
        assert_eq!(stored.room, None);
    }

    /// Formats arrived long after people had profiles. One written without the
    /// field has to open where it left them — 5v5 role queue was the only shape
    /// the app had — rather than in a queue they never chose.
    #[test]
    fn a_profile_written_before_formats_existed_starts_you_in_5v5() {
        let old = r#"{"role": "support", "support_pool": ["ana"]}"#;

        let stored: StoredProfile = serde_json::from_str(old).expect("the format is optional");
        assert_eq!(stored.format, None);

        let profile = Profile::from_stored(stored, &fixture());
        assert_eq!(profile.format, Format::default());
        assert_eq!(profile.format.team_size(), 5);
        assert_eq!(
            profile.pool(Role::Support).len(),
            1,
            "and the rest of it is untouched"
        );
    }

    /// Ranks arrived long after people had profiles. One written without the
    /// field has to open where it left them — the whole ladder, which is what
    /// this app scored on before the picker existed.
    #[test]
    fn a_profile_written_before_ranks_existed_reads_as_all_ranks() {
        let old = r#"{"role": "support", "support_pool": ["ana"]}"#;

        let stored: StoredProfile = serde_json::from_str(old).expect("the rank is optional");
        assert_eq!(stored.rank, None);

        let profile = Profile::from_stored(stored, &fixture());
        assert_eq!(profile.rank, Rank::All);
        assert_eq!(
            profile.pool(Role::Support).len(),
            1,
            "and the rest of it is untouched"
        );
    }

    #[test]
    fn the_rank_you_chose_is_the_one_you_come_back_to() {
        let dataset = fixture();
        let mut profile = Profile::empty(dataset.hero_count());
        profile.rank = Rank::Master;

        let raw = serde_json::to_string(&profile.to_stored(&dataset)).expect("serialises");
        assert!(
            raw.contains(r#""rank":"master""#),
            "the stable key is what goes on disk, got {raw}"
        );

        let stored: StoredProfile = serde_json::from_str(&raw).expect("round trips");
        assert_eq!(Profile::from_stored(stored, &dataset).rank, Rank::Master);
    }

    /// Why the rank is stored as a key rather than as the typed value the format
    /// beside it uses. Blizzard has inserted a division before (Emerald) and
    /// added one above (Champion); an unknown enum variant is a hard serde error
    /// on the whole struct, and `load` swallows that into a default — so a rung
    /// written by a newer build would cost an older one this person's pool,
    /// weights and session rather than just the setting.
    #[test]
    fn a_profile_naming_a_division_this_build_does_not_know_keeps_the_rest_of_it() {
        let future = r#"{"rank": "transcendent", "role": "support", "support_pool": ["ana"]}"#;

        let stored: StoredProfile = serde_json::from_str(future).expect("still parses");
        let profile = Profile::from_stored(stored, &fixture());

        assert_eq!(profile.rank, Rank::All, "the setting falls back");
        assert_eq!(profile.role, Role::Support, "and nothing else is lost");
        assert_eq!(profile.pool(Role::Support).len(), 1);
    }

    /// The guard against the pick column and the lock-top shortcut drifting
    /// apart: they build their context from this one function, so every field
    /// the scorer reads has to arrive through it.
    #[test]
    fn the_scoring_context_carries_every_profile_field_the_scorer_reads() {
        let dataset = fixture();
        let mut profile = Profile::empty(dataset.hero_count());
        profile.role = Role::Support;
        profile.rank = Rank::Diamond;
        profile.weights.rank = 0.42;
        profile.overrides[0] = 55;

        let ctx = profile.context(dataset.hero_count());
        assert_eq!(ctx.role, Role::Support);
        assert_eq!(ctx.rank, Rank::Diamond);
        assert_eq!(ctx.weights.rank, 0.42);
        assert_eq!(ctx.overrides[0], 55);
        assert_eq!(
            ctx.overrides.len(),
            dataset.hero_count(),
            "recommend() rejects any other length"
        );
    }

    #[test]
    fn the_format_you_chose_is_the_one_you_come_back_to() {
        let dataset = fixture();
        let mut profile = Profile::empty(dataset.hero_count());
        profile.format = Format::new(TeamSize::SixVSix, Queue::Open);

        let stored = profile.to_stored(&dataset);
        let raw = serde_json::to_string(&stored).expect("serialises");
        assert!(raw.contains(r#""size":"6v6""#), "{raw}");
        assert!(raw.contains(r#""queue":"open""#), "{raw}");

        let back: StoredProfile = serde_json::from_str(&raw).expect("deserialises");
        assert_eq!(
            Profile::from_stored(back, &dataset).format,
            profile.format,
            "a reload puts you back in the queue you were in"
        );
    }

    /// A profile whose session is blank means "alone", not "a session called
    /// nothing" — which would build a socket URL pointing at `/ws/`.
    #[test]
    fn a_blank_session_is_no_session() {
        let stored = StoredProfile {
            session: Some("   ".to_owned()),
            ..StoredProfile::default()
        };

        assert_eq!(Profile::from_stored(stored, &fixture()).session, None);
    }

    /// The smallest dataset `from_stored` will accept: it only ever looks
    /// heroes up by key, so the roster's contents are beside the point here.
    fn fixture() -> Dataset {
        use overwatch_core::{DatasetParts, Hero, Matrix};

        let heroes = vec![Hero {
            key: "ana".to_owned(),
            name: "Ana".to_owned(),
            role: Role::Support,
            subrole: None,
            aliases: vec!["ana".to_owned()],
        }];
        let n = heroes.len();

        Dataset::new(DatasetParts {
            heroes,
            maps: Vec::new(),
            matchups: Matrix::unrated(n),
            synergy: Matrix::unrated(n),
            map_affinity: Vec::new(),
            base_strength: vec![0; n],
            rank_shift: vec![[0; Rank::DIVISIONS.len()]; n],
            prevalence: vec![[0; Rank::CHOICES.len()]; n],
            win_rate: vec![None; n],
            side_lean: vec![0; n],
            shape: vec![[0; 3]; n],
            reasons: vec![String::new(); n * n],
            disputed: vec![false; n * n],
            generated: String::new(),
            patch: String::new(),
        })
        .expect("a one-hero dataset is valid")
    }
}
