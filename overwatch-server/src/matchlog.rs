//! Append-only log of match results.
//!
//! The point of recording outcomes is to eventually tune the personal
//! overrides: after enough games it becomes visible which "correct" picks you
//! actually lose on. That needs one append per match and a full read when
//! analysing — no queries, no indexes, no schema migrations.
//!
//! So this is JSON Lines rather than the SQLite the plan called for. Two people
//! logging a handful of matches a night will not outgrow it this decade, and it
//! stays greppable and diffable, which a binary database would not.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchRecord {
    /// Supplied by the client, which is the only side that knows the wall clock
    /// the players care about.
    pub at: String,
    pub player: String,
    pub role: String,
    pub hero: String,
    #[serde(default)]
    pub map: Option<String>,
    /// Which half of a payload map, where the mode has one. Defaulted so the
    /// lines written before this existed still read back.
    #[serde(default)]
    pub side: Option<String>,
    #[serde(default)]
    pub enemies: Vec<String>,
    #[serde(default)]
    pub allies: Vec<String>,
    /// `true` for a win. Deliberately not an enum: draws are vanishingly rare
    /// and a third state would complicate every consumer.
    pub won: bool,
}

#[derive(Clone)]
pub struct MatchLog {
    path: PathBuf,
}

impl MatchLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Appends one record. Opened per call rather than held open, so an
    /// external editor or a backup can move the file without the server
    /// silently writing into a deleted inode.
    pub async fn append(&self, record: &MatchRecord) -> Result<()> {
        let mut line = serde_json::to_string(record).context("serialising a match record")?;
        line.push('\n');

        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await
            .with_context(|| format!("opening {}", self.path.display()))?;

        file.write_all(line.as_bytes())
            .await
            .with_context(|| format!("appending to {}", self.path.display()))?;
        Ok(())
    }

    /// Reads every record, skipping any line that will not parse.
    ///
    /// A corrupt line — a half-written record after a power cut, say — costs
    /// that one match, not the whole history.
    pub async fn read_all(&self) -> Result<Vec<MatchRecord>> {
        let text = match tokio::fs::read_to_string(&self.path).await {
            Ok(text) => text,
            // No log yet simply means no matches played.
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(err).with_context(|| format!("reading {}", self.path.display()))
            }
        };

        Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<MatchRecord>(line).ok())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(hero: &str, won: bool) -> MatchRecord {
        MatchRecord {
            at: "2026-08-15T20:00:00Z".to_owned(),
            player: "me".to_owned(),
            role: "tank".to_owned(),
            hero: hero.to_owned(),
            map: Some("kings-row".to_owned()),
            side: Some("attack".to_owned()),
            enemies: vec!["pharah".to_owned()],
            allies: vec!["ana".to_owned()],
            won,
        }
    }

    /// A path no other test, run or process will pick.
    ///
    /// The pid alone is not enough. A test that fails skips the cleanup at its
    /// end, so the file outlives the run — and the next run to be handed that
    /// pid back inherits it. That makes these tests depend on the history of the
    /// machine they are on, which is how a suite develops a flake nobody can
    /// reproduce. The counter makes each path unique within a run and the clock
    /// makes it unique across them.
    fn temp_path(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_nanos())
            .unwrap_or(0);

        std::env::temp_dir().join(format!(
            "overwatch-matchlog-{name}-{}-{nonce}-{}.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ))
    }

    #[tokio::test]
    async fn records_round_trip() {
        let path = temp_path("round-trip");
        let _ = tokio::fs::remove_file(&path).await;
        let log = MatchLog::new(path.clone());

        log.append(&record("reinhardt", true))
            .await
            .expect("append");
        log.append(&record("dva", false)).await.expect("append");

        let all = log.read_all().await.expect("read");
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], record("reinhardt", true));
        assert!(!all[1].won);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn a_missing_log_is_simply_empty() {
        let log = MatchLog::new(temp_path("absent"));
        assert!(log.read_all().await.expect("read").is_empty());
    }

    #[tokio::test]
    async fn a_corrupt_line_costs_only_that_record() {
        let path = temp_path("corrupt");
        let _ = tokio::fs::remove_file(&path).await;
        let log = MatchLog::new(path.clone());

        log.append(&record("reinhardt", true))
            .await
            .expect("append");
        tokio::fs::write(
            &path,
            format!(
                "{}\n{{ truncated\n{}\n",
                serde_json::to_string(&record("reinhardt", true)).expect("json"),
                serde_json::to_string(&record("sigma", false)).expect("json"),
            ),
        )
        .await
        .expect("write");

        let all = log.read_all().await.expect("read");
        assert_eq!(all.len(), 2, "the good records survive");

        let _ = tokio::fs::remove_file(&path).await;
    }
}
