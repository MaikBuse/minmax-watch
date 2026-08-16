use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// Dense index into the roster. Keeping heroes as a small integer is what lets
/// the matchup matrix be one flat allocation instead of a map of maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HeroId(pub u16);

impl HeroId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Tank,
    Damage,
    Support,
}

/// Tank, matching the role a fresh profile starts on. A default only exists so
/// that a session seat arriving without one is readable rather than an error —
/// it is never a claim about what anybody plays.
impl Default for Role {
    fn default() -> Self {
        Self::Tank
    }
}

impl Role {
    /// The roles this app has pick modes for, in the order the mode switch
    /// walks them. Every role is playable; the constant stays separate from
    /// [`Role::ALL`] because one is "what you can pick as" and the other is
    /// "what a team is made of", and they are free to diverge again.
    pub const PLAYABLE_MODES: [Role; 3] = [Role::Tank, Role::Damage, Role::Support];

    /// Every role, in the order a team is drafted. Also the index order used by
    /// anything that stores one value per role.
    pub const ALL: [Role; 3] = [Role::Tank, Role::Damage, Role::Support];

    /// Position in [`Role::ALL`], for indexing per-role tables.
    pub const fn index(self) -> usize {
        match self {
            Role::Tank => 0,
            Role::Damage => 1,
            Role::Support => 2,
        }
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tank" => Ok(Role::Tank),
            "damage" | "dps" | "offense" => Ok(Role::Damage),
            "support" | "healer" => Ok(Role::Support),
            other => Err(CoreError::UnknownRole(other.to_owned())),
        }
    }

    /// The stable key: what [`Role::parse`] round-trips, and what the stored
    /// profile and the match log are written with. Not what the screen shows —
    /// see [`Role::label`].
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Tank => "tank",
            Role::Damage => "damage",
            Role::Support => "support",
        }
    }

    /// The word people say mid-draft: "dps", not "damage".
    ///
    /// Display only. Kept apart from [`Role::as_str`] so that renaming what is
    /// on screen can never change what is on disk.
    pub const fn label(self) -> &'static str {
        match self {
            Role::Tank => "tank",
            Role::Damage => "dps",
            Role::Support => "support",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hero {
    /// Stable slug, matching the OverFast API key (e.g. `wrecking-ball`).
    pub key: String,
    pub name: String,
    pub role: Role,
    /// Short forms typed during a draft: `rein`, `rh`, `hog`, `ball`, `jq`.
    /// These are what make a pick resolvable in one or two keystrokes.
    #[serde(default)]
    pub aliases: Vec<String>,
}

/// A set of heroes, used for pools and for "already picked" checks.
///
/// Backed by four `u64` words, so it is `Copy`, allocation-free, and cheap to
/// test in the inner scoring loop. Caps the roster at 256, which is comfortable
/// given the live roster is 53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HeroSet {
    words: [u64; Self::WORDS],
}

impl HeroSet {
    const WORDS: usize = 4;
    pub const CAPACITY: usize = Self::WORDS * 64;

    pub const fn empty() -> Self {
        Self {
            words: [0; Self::WORDS],
        }
    }

    /// Fails rather than silently dropping a hero that does not fit.
    pub fn insert(&mut self, hero: HeroId) -> Result<(), CoreError> {
        let idx = hero.index();
        if idx >= Self::CAPACITY {
            return Err(CoreError::RosterTooLarge(idx + 1, Self::CAPACITY));
        }
        self.words[idx / 64] |= 1u64 << (idx % 64);
        Ok(())
    }

    pub fn remove(&mut self, hero: HeroId) {
        let idx = hero.index();
        if idx < Self::CAPACITY {
            self.words[idx / 64] &= !(1u64 << (idx % 64));
        }
    }

    pub fn contains(&self, hero: HeroId) -> bool {
        let idx = hero.index();
        idx < Self::CAPACITY && (self.words[idx / 64] >> (idx % 64)) & 1 == 1
    }

    pub fn toggle(&mut self, hero: HeroId) -> Result<(), CoreError> {
        if self.contains(hero) {
            self.remove(hero);
            Ok(())
        } else {
            self.insert(hero)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|w| *w == 0)
    }

    pub fn len(&self) -> usize {
        self.words.iter().map(|w| w.count_ones() as usize).sum()
    }

    pub fn iter(&self) -> impl Iterator<Item = HeroId> + '_ {
        (0..Self::CAPACITY)
            .map(|i| HeroId(i as u16))
            .filter(move |h| self.contains(*h))
    }

    pub fn from_iter_checked<I: IntoIterator<Item = HeroId>>(iter: I) -> Result<Self, CoreError> {
        let mut set = Self::empty();
        for hero in iter {
            set.insert(hero)?;
        }
        Ok(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hero_set_round_trips() {
        let mut set = HeroSet::empty();
        assert!(set.is_empty());

        set.insert(HeroId(0)).expect("0 fits");
        set.insert(HeroId(52)).expect("52 fits");
        set.insert(HeroId(200)).expect("200 fits");

        assert!(set.contains(HeroId(0)));
        assert!(set.contains(HeroId(52)));
        assert!(set.contains(HeroId(200)));
        assert!(!set.contains(HeroId(1)));
        assert_eq!(set.len(), 3);

        set.remove(HeroId(52));
        assert!(!set.contains(HeroId(52)));
        assert_eq!(set.len(), 2);

        let collected: Vec<_> = set.iter().collect();
        assert_eq!(collected, vec![HeroId(0), HeroId(200)]);
    }

    #[test]
    fn hero_set_rejects_out_of_capacity() {
        let mut set = HeroSet::empty();
        assert_eq!(
            set.insert(HeroId(256)),
            Err(CoreError::RosterTooLarge(257, 256))
        );
    }

    /// The gate that used to hold damage out. Asserted so it cannot close again
    /// by accident, and so that every playable mode is a role the roster
    /// actually has heroes for.
    #[test]
    fn every_role_is_playable_and_labelled() {
        for role in Role::ALL {
            assert!(
                Role::PLAYABLE_MODES.contains(&role),
                "{role:?} has no pick mode"
            );
            // The stable key round-trips; the label is display only and is free
            // to differ, which is exactly what "dps" is.
            assert_eq!(Role::parse(role.as_str()), Ok(role));
            assert!(!role.label().is_empty());
        }
        assert_eq!(Role::parse(Role::Damage.label()), Ok(Role::Damage));
    }

    #[test]
    fn role_parsing_accepts_common_spellings() {
        assert_eq!(Role::parse("Tank"), Ok(Role::Tank));
        assert_eq!(Role::parse("dps"), Ok(Role::Damage));
        assert_eq!(Role::parse("  SUPPORT "), Ok(Role::Support));
        assert!(Role::parse("jungler").is_err());
    }
}
