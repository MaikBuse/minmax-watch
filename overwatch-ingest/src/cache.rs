//! On-disk cache of every raw response the ingest has ever fetched.
//!
//! Two reasons this exists. First, re-parsing is the part that changes often
//! (site markup shifts, our extraction improves) and re-fetching 150+ pages to
//! test a selector tweak is both slow and rude. Second, the cached HTML is the
//! audit trail: when a matchup value looks wrong, the exact bytes it came from
//! are still on disk.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

/// Identify ourselves honestly rather than pretending to be a browser.
const USER_AGENT: &str = concat!(
    "minmax-watch/",
    env!("CARGO_PKG_VERSION"),
    " (draft assistant; +https://minmax.watch; contact: maik.buse@sicore.de)"
);

/// Politeness delay between live requests to the same host.
const REQUEST_DELAY: Duration = Duration::from_millis(1100);

pub struct Fetcher {
    client: reqwest::Client,
    cache_dir: PathBuf,
    /// When set, ignore cached copies and re-fetch everything.
    refresh: bool,
    live_requests: usize,
}

impl Fetcher {
    pub fn new(cache_dir: PathBuf, refresh: bool) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(30))
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            client,
            cache_dir,
            refresh,
            live_requests: 0,
        })
    }

    pub fn live_requests(&self) -> usize {
        self.live_requests
    }

    fn path_for(&self, slug: &str) -> PathBuf {
        self.cache_dir.join(slug)
    }

    /// Drops a cached entry.
    ///
    /// Used when a response turns out to be useless — a "Hero Not Found" page
    /// served as HTTP 200, say — so the next candidate slug can be tried under
    /// the same cache key.
    pub async fn forget(&self, slug: &str) {
        let _ = tokio::fs::remove_file(self.path_for(slug)).await;
    }

    fn miss_path(&self, slug: &str) -> PathBuf {
        self.path_for(&format!("{slug}.missing"))
    }

    /// Records that a source genuinely has no page for this entry.
    ///
    /// Without this, a hero the site has never heard of costs a request on
    /// every single run forever. Call it only after every candidate slug has
    /// been tried, since the cache key is shared across candidates.
    pub async fn mark_missing(&self, slug: &str) {
        if let Some(parent) = self.miss_path(slug).parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let _ = tokio::fs::write(self.miss_path(slug), b"").await;
    }

    /// Whether a previous run proved this entry absent. Always false under
    /// `--refresh`, which is how a newly added hero gets picked up.
    pub async fn is_missing(&self, slug: &str) -> bool {
        !self.refresh
            && tokio::fs::try_exists(self.miss_path(slug))
                .await
                .unwrap_or(false)
    }

    /// Returns the body for `url`, from cache when present.
    ///
    /// `slug` is the cache filename and must be stable across runs - it is what
    /// makes a second `just ingest` a no-op.
    pub async fn get(&mut self, url: &str, slug: &str) -> Result<String> {
        let path = self.path_for(slug);

        if !self.refresh {
            if let Ok(body) = tokio::fs::read_to_string(&path).await {
                if !body.is_empty() {
                    return Ok(body);
                }
            }
        }

        if self.live_requests > 0 {
            tokio::time::sleep(REQUEST_DELAY).await;
        }
        self.live_requests += 1;

        eprintln!("  fetch {url}");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("{url} returned HTTP {status}");
        }

        let body = response
            .text()
            .await
            .with_context(|| format!("reading the body of {url}"))?;

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        tokio::fs::write(&path, &body)
            .await
            .with_context(|| format!("caching {}", path.display()))?;

        // A slug that used to be absent and now answers is no longer missing.
        let _ = tokio::fs::remove_file(self.miss_path(slug)).await;

        Ok(body)
    }

    /// Like [`Fetcher::get`], but for responses that are not text.
    ///
    /// Deliberately not implemented in terms of `get`: that one goes through
    /// `response.text()`, whose charset decoding is what makes the scraped HTML
    /// come out right and what would corrupt a PNG.
    ///
    /// Returns `Ok(None)` when the server says the file does not exist, and
    /// remembers that so later runs do not re-ask. An index that advertises an
    /// image nobody uploaded is normal here, and it is not a reason to fail.
    /// Any other failure is still an error.
    pub async fn get_bytes(&mut self, url: &str, slug: &str) -> Result<Option<Vec<u8>>> {
        if self.is_missing(slug).await {
            return Ok(None);
        }

        let path = self.path_for(slug);

        if !self.refresh {
            if let Ok(bytes) = tokio::fs::read(&path).await {
                if !bytes.is_empty() {
                    return Ok(Some(bytes));
                }
            }
        }

        if self.live_requests > 0 {
            tokio::time::sleep(REQUEST_DELAY).await;
        }
        self.live_requests += 1;

        eprintln!("  fetch {url}");
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("requesting {url}"))?;

        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            self.mark_missing(slug).await;
            return Ok(None);
        }
        if !status.is_success() {
            anyhow::bail!("{url} returned HTTP {status}");
        }

        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("reading the body of {url}"))?
            .to_vec();

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        tokio::fs::write(&path, &bytes)
            .await
            .with_context(|| format!("caching {}", path.display()))?;

        let _ = tokio::fs::remove_file(self.miss_path(slug)).await;

        Ok(Some(bytes))
    }
}

/// Writes `contents` to `path` only when it differs, so an unchanged ingest run
/// leaves file timestamps - and therefore rebuilds - untouched.
pub async fn write_if_changed(path: &Path, contents: &str) -> Result<bool> {
    if let Ok(existing) = tokio::fs::read_to_string(path).await {
        if existing == contents {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    tokio::fs::write(path, contents)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// The binary counterpart of [`write_if_changed`].
///
/// Re-encoding an image is deterministic, so an unchanged source produces
/// identical bytes and this leaves the file — and the git diff — alone.
pub async fn write_bytes_if_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    if let Ok(existing) = tokio::fs::read(path).await {
        if existing == contents {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    tokio::fs::write(path, contents)
        .await
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}
