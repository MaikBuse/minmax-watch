//! Which rung of the competitive ladder the patch-strength numbers were read on.
//!
//! A property of the *population a number was measured over*, not of a hero and
//! not of the board — which is why it lives here rather than in [`crate::hero`]
//! or [`crate::format`].
//!
//! **Nothing about a *pair* is sliced this way, and nothing can be.** The
//! counter, synergy, map, side and shape terms read the same numbers at every
//! rung, because no source publishes them any other way: counterwatch's rank
//! filter is not URL-addressable and its counters and duos pages carry no
//! per-division breakdown at all, and Blizzard's rates endpoint publishes one row
//! per hero and never a pair. Rank-slicing the matchup matrix would also mean
//! eight copies of a 322 KB file inside a wasm bundle built at `opt-level = "s"`.
//! Anything that makes it look like a rank changes a matchup is wrong.
//!
//! What *is* sliced is the two things published per hero per rung: patch strength
//! (as a shift, on eight columns indexed by [`Rank::column`]) and prevalence (as a
//! reading, on nine columns indexed by [`Rank::slot`]).

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// One rung of the competitive ladder, plus the whole-ladder aggregate.
///
/// Eight rungs and not nine: Grandmaster and Champion are reported together by
/// both sources — Blizzard as `tier=Grandmaster`, counterwatch as
/// `Grandmaster+` — and neither publishes them apart, so this is the finest
/// slice that exists.
///
/// [`Rank::All`] is a real reading rather than an absence: it is the column
/// `strength.toml` has always held, and the one both sources publish beside the
/// rungs. That is why this is not an `Option<Rank>` anywhere — unlike
/// [`crate::Matrix::rating`], where "nothing known" and "dead even" are
/// genuinely different claims, here "the player has not chosen" and "the whole
/// ladder" are the same number read the same way.
///
/// `Ord` derives on purpose and is load-bearing: the ladder is ordered, the
/// per-rank tables are laid out on [`Rank::DIVISIONS`], and the ingest's
/// smoothing pass calls two rungs adjacent because they are adjacent here.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Rank {
    /// Every division at once — how this app scored before ranks existed, and
    /// what a profile written before them still reads as.
    #[default]
    All,
    Bronze,
    Silver,
    Gold,
    Platinum,
    Emerald,
    Diamond,
    Master,
    /// Grandmaster *and* Champion. See [`Rank::label`].
    Grandmaster,
}

impl Rank {
    /// Every choice the picker offers, in the order it walks them, the aggregate
    /// first.
    ///
    /// Kept apart from [`Rank::DIVISIONS`] for the same reason
    /// [`crate::Role::PLAYABLE_MODES`] is kept apart from [`crate::Role::ALL`]:
    /// one is "what you can choose" and the other is "what the data has a column
    /// for", and they are free to diverge.
    pub const CHOICES: [Rank; 9] = [
        Rank::All,
        Rank::Bronze,
        Rank::Silver,
        Rank::Gold,
        Rank::Platinum,
        Rank::Emerald,
        Rank::Diamond,
        Rank::Master,
        Rank::Grandmaster,
    ];

    /// The eight rungs the sources publish a column for, low to high. Also the
    /// index order of anything that stores one value per rung.
    ///
    /// Deliberately **not** named `ALL`. `Rank::ALL` sitting beside `Rank::All`
    /// is an off-by-one waiting to happen, and a per-rank table indexed one row
    /// out loads, scores and passes every count-based test.
    pub const DIVISIONS: [Rank; 8] = [
        Rank::Bronze,
        Rank::Silver,
        Rank::Gold,
        Rank::Platinum,
        Rank::Emerald,
        Rank::Diamond,
        Rank::Master,
        Rank::Grandmaster,
    ];

    /// Position in [`Rank::CHOICES`], for indexing a table that has a column for
    /// the aggregate as well as the rungs. [`Rank::All`] is **0**.
    ///
    /// Deliberately a different index space from [`Self::column`], and the two
    /// must never be crossed: a nine-wide table read with `column()` is off by
    /// one for every rung and a shift table read with `slot()` runs off the end
    /// of the last one. Both load, score and pass every count-based test.
    ///
    /// Which one a table wants follows from what its numbers *are*. A shift is
    /// measured against the aggregate, so the aggregate cannot have a column and
    /// `column()` returns `None` there. A pick rate at all ranks is a published
    /// figure like any other rung's, so it gets a column and `slot()` names it.
    pub const fn slot(self) -> usize {
        match self.column() {
            Some(column) => column + 1,
            None => 0,
        }
    }

    /// Position in [`Rank::DIVISIONS`], for indexing per-rank tables. `None` for
    /// [`Rank::All`], which has no column of its own — it is what the table is
    /// measured against.
    pub const fn column(self) -> Option<usize> {
        match self {
            Rank::All => None,
            Rank::Bronze => Some(0),
            Rank::Silver => Some(1),
            Rank::Gold => Some(2),
            Rank::Platinum => Some(3),
            Rank::Emerald => Some(4),
            Rank::Diamond => Some(5),
            Rank::Master => Some(6),
            Rank::Grandmaster => Some(7),
        }
    }

    /// Reads a rung from any of the spellings the sources and the stored profile
    /// use.
    ///
    /// The trailing `+` is stripped rather than special-cased because this one
    /// function has to read three different spellings of the same bucket:
    /// counterwatch's `Grandmaster+` cell, Blizzard's `Grandmaster`, and the
    /// `grandmaster` key on disk. Same trick [`crate::Role::parse`] plays with
    /// `dps`.
    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let cleaned = s.trim().trim_end_matches('+').trim().to_ascii_lowercase();
        match cleaned.as_str() {
            "all" | "all ranks" | "all-ranks" | "all tiers" => Ok(Rank::All),
            "bronze" => Ok(Rank::Bronze),
            "silver" => Ok(Rank::Silver),
            "gold" => Ok(Rank::Gold),
            "platinum" | "plat" => Ok(Rank::Platinum),
            "emerald" => Ok(Rank::Emerald),
            "diamond" => Ok(Rank::Diamond),
            "master" | "masters" => Ok(Rank::Master),
            // Champion has no column of its own anywhere; both sources fold it
            // into the top bucket, so this is where somebody who types it lands.
            "grandmaster" | "gm" | "champion" | "champ" => Ok(Rank::Grandmaster),
            other => Err(CoreError::UnknownRank(other.to_owned())),
        }
    }

    /// The stable key: what [`Rank::parse`] round-trips, what the stored profile
    /// and the session seat are written with, and what the columns in
    /// `strength_by_rank.toml` are named. Not what the screen shows — see
    /// [`Rank::label`].
    pub const fn as_str(self) -> &'static str {
        match self {
            Rank::All => "all",
            Rank::Bronze => "bronze",
            Rank::Silver => "silver",
            Rank::Gold => "gold",
            Rank::Platinum => "platinum",
            Rank::Emerald => "emerald",
            Rank::Diamond => "diamond",
            Rank::Master => "master",
            Rank::Grandmaster => "grandmaster",
        }
    }

    /// What the chip and the reason line show.
    ///
    /// Grandmaster gains a `+` here and only here, because the bucket really is
    /// Grandmaster *and* Champion and a label that said only "grandmaster" would
    /// be leaving out half of who is in it. [`Rank::as_str`] must not carry the
    /// `+`: that string is a TOML column name and a value in everybody's stored
    /// profile.
    pub const fn label(self) -> &'static str {
        match self {
            Rank::All => "all ranks",
            Rank::Bronze => "bronze",
            Rank::Silver => "silver",
            Rank::Gold => "gold",
            Rank::Platinum => "platinum",
            Rank::Emerald => "emerald",
            Rank::Diamond => "diamond",
            Rank::Master => "master",
            Rank::Grandmaster => "grandmaster+",
        }
    }

    /// The longer form, for a `title` and an accessible name — the same job
    /// [`crate::Queue::description`] does.
    pub const fn description(self) -> &'static str {
        match self {
            Rank::All => "every division, as this app has always scored",
            Rank::Bronze => "bronze",
            Rank::Silver => "silver",
            Rank::Gold => "gold",
            Rank::Platinum => "platinum",
            Rank::Emerald => "emerald",
            Rank::Diamond => "diamond",
            Rank::Master => "master",
            Rank::Grandmaster => "grandmaster and champion",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The array layout of every per-rank table rests on this, and so does the
    /// ingest's claim that it is smoothing across *adjacent* populations. Both
    /// fail silently if the order ever drifts.
    #[test]
    fn the_ladder_runs_from_bronze_to_grandmaster_in_order() {
        assert_eq!(Rank::DIVISIONS.len(), 8);
        assert_eq!(Rank::CHOICES.len(), Rank::DIVISIONS.len() + 1);
        assert_eq!(
            Rank::CHOICES[0],
            Rank::All,
            "the aggregate leads the picker"
        );

        for (index, rank) in Rank::DIVISIONS.iter().enumerate() {
            assert_eq!(
                rank.column(),
                Some(index),
                "{rank:?} indexes its own column"
            );
        }
        for pair in Rank::DIVISIONS.windows(2) {
            assert!(
                pair[0] < pair[1],
                "{:?} must sort below {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(Rank::All.column(), None, "the aggregate has no column");
    }

    /// The two index spaces, pinned apart. Crossing them produces a table that is
    /// off by one for every rung and passes every count-based test.
    #[test]
    fn the_nine_wide_slot_is_a_different_index_from_the_eight_wide_column() {
        assert_eq!(
            Rank::All.slot(),
            0,
            "the aggregate leads, and it has a slot"
        );

        for (index, rank) in Rank::CHOICES.iter().enumerate() {
            assert_eq!(rank.slot(), index, "{rank:?} indexes its own slot");
        }
        for rank in Rank::DIVISIONS {
            assert_eq!(
                rank.slot(),
                rank.column().expect("a division has a column") + 1,
                "{rank:?} sits one along, because the aggregate took the first slot"
            );
        }
    }

    /// These strings are in every stored profile, on the session wire, and are
    /// the column names of a committed file. Renaming one silently resets the
    /// setting for everybody and orphans a column.
    #[test]
    fn a_rank_round_trips_through_the_names_it_is_stored_under() {
        for rank in Rank::CHOICES {
            assert_eq!(Rank::parse(rank.as_str()), Ok(rank));
            assert_eq!(Rank::parse(rank.label()), Ok(rank));
        }
        assert_eq!(Rank::All.as_str(), "all");
        assert_eq!(Rank::Grandmaster.as_str(), "grandmaster");
        assert_eq!(Rank::Grandmaster.label(), "grandmaster+");
    }

    /// The two sources spell the top bucket differently and the ingest reads
    /// both through this one function.
    #[test]
    fn both_sources_spellings_of_the_top_bucket_land_on_one_rung() {
        assert_eq!(Rank::parse("Grandmaster+"), Ok(Rank::Grandmaster));
        assert_eq!(Rank::parse("Grandmaster"), Ok(Rank::Grandmaster));
        assert_eq!(Rank::parse("champion"), Ok(Rank::Grandmaster));
        assert_eq!(Rank::parse(" GM "), Ok(Rank::Grandmaster));
    }

    #[test]
    fn an_unset_rank_is_the_whole_ladder() {
        assert_eq!(Rank::default(), Rank::All);
    }

    #[test]
    fn a_division_this_app_does_not_know_is_an_error_rather_than_a_guess() {
        assert!(Rank::parse("transcendent").is_err());
        assert!(Rank::parse("").is_err());
    }
}
