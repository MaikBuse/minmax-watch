use serde::{Deserialize, Serialize};

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MapId(pub u16);

impl MapId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// The game modes that appear in competitive play. OverFast also reports
/// arcade, workshop and deathmatch modes; the ingest filters those out because
/// we never draft on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GameMode {
    Control,
    Escort,
    Hybrid,
    Push,
    Flashpoint,
    Clash,
}

impl GameMode {
    pub const COMPETITIVE: [GameMode; 6] = [
        GameMode::Control,
        GameMode::Escort,
        GameMode::Hybrid,
        GameMode::Push,
        GameMode::Flashpoint,
        GameMode::Clash,
    ];

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        match s.trim().to_ascii_lowercase().as_str() {
            "control" => Ok(GameMode::Control),
            "escort" => Ok(GameMode::Escort),
            "hybrid" => Ok(GameMode::Hybrid),
            "push" => Ok(GameMode::Push),
            "flashpoint" => Ok(GameMode::Flashpoint),
            "clash" => Ok(GameMode::Clash),
            other => Err(CoreError::UnknownGameMode(other.to_owned())),
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            GameMode::Control => "control",
            GameMode::Escort => "escort",
            GameMode::Hybrid => "hybrid",
            GameMode::Push => "push",
            GameMode::Flashpoint => "flashpoint",
            GameMode::Clash => "clash",
        }
    }

    /// Whether one team attacks and the other defends.
    ///
    /// Only the payload modes. Push, Control, Flashpoint and Clash start both
    /// teams in the same posture, so asking which side you are on is a question
    /// with no answer — which is why the UI does not render the toggle for them
    /// rather than defaulting it.
    pub const fn has_sides(self) -> bool {
        matches!(self, GameMode::Escort | GameMode::Hybrid)
    }
}

/// Which half of a payload map you are playing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Attack,
    Defend,
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Attack, Side::Defend];

    /// Sign applied to a hero's attack-leaning score. Defence is the same scale
    /// read backwards, so one number per hero covers both sides.
    pub const fn sign(self) -> f32 {
        match self {
            Side::Attack => 1.0,
            Side::Defend => -1.0,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Side::Attack => "attack",
            Side::Defend => "defend",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_payload_modes_have_sides() {
        assert!(GameMode::Escort.has_sides());
        assert!(GameMode::Hybrid.has_sides());
        for mode in [
            GameMode::Control,
            GameMode::Push,
            GameMode::Flashpoint,
            GameMode::Clash,
        ] {
            assert!(!mode.has_sides(), "{mode:?} is symmetric");
        }
    }

    #[test]
    fn the_two_sides_read_one_scale_in_opposite_directions() {
        assert_eq!(Side::Attack.sign(), -Side::Defend.sign());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMap {
    pub key: String,
    pub name: String,
    pub mode: GameMode,
    #[serde(default)]
    pub aliases: Vec<String>,
}
