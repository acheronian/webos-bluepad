//! The check-only update flow (spec: Update behavior; glossary: Update
//! check, Tombstone manifest).
//!
//! The UI's courtesy look at the app's release manifest — informational
//! only, reporting and pointing to the Homebrew Channel, never installing.
//! The connection daemon takes no part: the check lives in the UI process,
//! which is why the manifest fetcher is a trait — tests inject fakes, no
//! network in unit tests.

use serde::{Deserialize, Serialize};

/// Whether `latest` is a newer dotted version than `current` (the HBC
/// manifest shape: plain `major.minor.patch`, an optional leading `v`).
/// Numeric segment-wise comparison, so `0.10.0` beats `0.9.0`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    let segments = |version: &str| {
        version
            .trim_start_matches('v')
            .split('.')
            .map(|segment| segment.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (latest, current) = (segments(latest), segments(current));
    for index in 0..latest.len().max(current.len()) {
        let (left, right) = (
            latest.get(index).copied().unwrap_or(0),
            current.get(index).copied().unwrap_or(0),
        );
        if left != right {
            return left > right;
        }
    }
    false
}

/// A release manifest as the Homebrew Channel serves it — the app's own
/// GitHub release manifest, or a tombstone manifest announcing the channel
/// is discontinued and naming the successor (glossary: Tombstone manifest).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReleaseManifest {
    /// The app id the manifest describes (informational; we don't act on it).
    pub id: Option<String>,
    /// The released version, absent on a tombstone.
    pub version: Option<String>,
    /// True when this channel is discontinued.
    pub discontinued: bool,
    /// Where installed versions reroute their update check after a host move.
    pub successor: Option<String>,
    /// The human-facing discontinuation message to surface.
    pub message: Option<String>,
}

impl ReleaseManifest {
    /// Parse a manifest document; unknown fields are ignored for forward
    /// compatibility (the settings-document precedent).
    pub fn parse(text: &str) -> Result<ReleaseManifest, UpdateError> {
        serde_json::from_str(text).map_err(|error| UpdateError::Parse(error.to_string()))
    }
}

/// The seam the whole flow is tested at: fetch a manifest URL's document.
/// The real implementation speaks HTTPS; tests inject canned pages.
pub trait ManifestFetcher {
    fn fetch(&self, url: &str) -> Result<String, UpdateError>;
}

/// The result of a courtesy look at the release manifest — reporting
/// only, never a trigger to install (glossary: Update check).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateReport {
    pub current_version: String,
    pub latest_version: String,
    /// True when the manifest names a newer version than this build.
    pub update_available: bool,
    /// The manifest URL the check actually read (post-successor).
    pub manifest_url: String,
    /// The discontinuation message surfaced when a tombstone was followed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone_message: Option<String>,
}

/// Perform the check against one manifest URL: fetch, parse, compare.
fn check_one(
    fetcher: &dyn ManifestFetcher,
    url: &str,
    current_version: &str,
) -> Result<(ReleaseManifest, UpdateReport), UpdateError> {
    let manifest = ReleaseManifest::parse(&fetcher.fetch(url)?)?;
    let latest_version = manifest.version.clone().unwrap_or_default();
    let report = UpdateReport {
        current_version: current_version.to_string(),
        update_available: is_newer(&latest_version, current_version),
        latest_version,
        manifest_url: url.to_string(),
        tombstone_message: None,
    };
    Ok((manifest, report))
}

/// The courtesy look: read the release manifest and report whether a newer
/// version exists, following a tombstone manifest to its successor channel
/// and surfacing its message (glossary: Tombstone manifest).
pub fn check_update(
    fetcher: &dyn ManifestFetcher,
    manifest_url: &str,
    current_version: &str,
) -> Result<UpdateReport, UpdateError> {
    let (manifest, mut report) = check_one(fetcher, manifest_url, current_version)?;
    if !manifest.discontinued {
        return Ok(report);
    }
    let successor = manifest.successor.clone();
    let message = manifest.message.clone();
    let (successor_manifest, mut successor_report) =
        check_one(fetcher, &successor.ok_or(UpdateError::NoSuccessor)?, current_version)?;
    successor_report.tombstone_message = message;
    if successor_manifest.discontinued {
        return Err(UpdateError::TombstoneLoop);
    }
    report = successor_report;
    Ok(report)
}

/// The UI's own state document — kept beside the daemon's settings, but
/// owned by the UI alone. Holds everything the update flow persists: the
/// manual update-source override, the 24 h-throttled last-check timestamp,
/// and the cached result of the last cold-start check (spec: Update
/// behavior; the daemon's settings document is never touched).
/// The UI state document's schema version — bumped on breaking change;
/// documents from a newer schema are renamed `.bad`, never obeyed (the
/// settings-document precedent).
pub const UI_STATE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct UiStateDocument {
    schema_version: u32,
    /// The advanced manual override of the manifest URL (glossary:
    /// Update check — the maintainer's fallback when the feed moves).
    update_source: Option<String>,
    /// When the last cold-start check ran (unix seconds).
    last_update_check: Option<u64>,
    /// The last successful check's report, shown until the next check.
    cached_update: Option<UpdateReport>,
}

/// The UI-owned state store: whole-document load/save in the settings
/// store's image — atomic writes, `.bad` rename on unparseable documents,
/// defaults on absence.
pub struct UiStateStore {
    path: std::path::PathBuf,
    doc: UiStateDocument,
}

/// How often the cold-start check may run (spec: 24 h-throttled).
pub const UPDATE_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;

impl Default for UiStateDocument {
    fn default() -> Self {
        UiStateDocument {
            schema_version: UI_STATE_SCHEMA_VERSION,
            update_source: None,
            last_update_check: None,
            cached_update: None,
        }
    }
}

impl UiStateStore {
    /// Load the UI state; an absent, unparseable, or future-schema document
    /// falls back to defaults, the bad one renamed `.bad` for inspection.
    pub fn load(path: impl Into<std::path::PathBuf>) -> UiStateStore {
        let path = path.into();
        let doc = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| match serde_json::from_str::<UiStateDocument>(&text) {
                Ok(doc) if doc.schema_version <= UI_STATE_SCHEMA_VERSION => Some(doc),
                _ => {
                    let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                    None
                }
            })
            .unwrap_or_default();
        UiStateStore { doc, path }
    }

    /// Persist atomically: temp file next to the target, then rename.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.doc)?)?;
        std::fs::rename(&tmp, &self.path)
    }

    /// The manifest URL to check: the manual override, or the default feed.
    pub fn effective_manifest_url(&self) -> &str {
        self.doc
            .update_source
            .as_deref()
            .unwrap_or(DEFAULT_MANIFEST_URL)
    }

    pub fn update_source(&self) -> Option<&str> {
        self.doc.update_source.as_deref()
    }

    pub fn set_update_source(&mut self, url: Option<String>) -> std::io::Result<()> {
        self.doc.update_source = url;
        self.save()
    }

    pub fn cached_update(&self) -> Option<&UpdateReport> {
        self.doc.cached_update.as_ref()
    }

    /// Whether the cold-start check may run at `now` — at most once per
    /// 24 h (spec: Update behavior).
    pub fn update_check_due(&self, now: u64) -> bool {
        self.doc
            .last_update_check
            .map(|last| now - last >= UPDATE_CHECK_INTERVAL_SECS)
            .unwrap_or(true)
    }

    /// Record a successful check: cache its report and stamp the time.
    pub fn record_update_check(&mut self, report: UpdateReport, now: u64) -> std::io::Result<()> {
        self.doc.cached_update = Some(report);
        self.record_update_attempt(now)
    }

    /// Stamp that a check ran at `now`, keeping the last cached result —
    /// a failed check still counts against the 24 h throttle.
    pub fn record_update_attempt(&mut self, now: u64) -> std::io::Result<()> {
        self.doc.last_update_check = Some(now);
        self.save()
    }
}

/// The real fetcher: HTTPS with a short timeout, so a dead network never
/// holds the UI hostage. The one piece the unit tests never touch — it is
/// a thin adapter over `ureq`, everything interesting sits above the seam.
pub struct HttpManifestFetcher {
    agent: ureq::Agent,
}

impl HttpManifestFetcher {
    pub fn new() -> Self {
        HttpManifestFetcher {
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(10))
                .build(),
        }
    }
}

impl Default for HttpManifestFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ManifestFetcher for HttpManifestFetcher {
    fn fetch(&self, url: &str) -> Result<String, UpdateError> {
        let response = self
            .agent
            .get(url)
            .call()
            .map_err(|error| UpdateError::Fetch(error.to_string()))?;
        response
            .into_string()
            .map_err(|error| UpdateError::Fetch(error.to_string()))
    }
}

/// The cold-start check: run it when due, serve the cached result inside
/// the 24 h window, and stay silent on failure (spec: Update behavior).
/// The attempt is stamped even when offline — the throttle limits attempts,
/// not successes, so the feed is queried at most once per 24 h.
pub fn cold_start_check(
    store: &mut UiStateStore,
    fetcher: &dyn ManifestFetcher,
    now: u64,
    current_version: &str,
) -> Option<String> {
    if !store.update_check_due(now) {
        return store.cached_update().and_then(render_cold_start_banner);
    }
    match check_update(fetcher, store.effective_manifest_url(), current_version) {
        Ok(report) => {
            let banner = render_cold_start_banner(&report);
            let _ = store.record_update_check(report, now);
            banner
        }
        Err(_) => {
            let _ = store.record_update_attempt(now);
            None
        }
    }
}

/// The line(s) the cold-start check may add to the main view: a banner
/// only when an update is available, the tombstone's message first when
/// one was followed. `None` when there is nothing worth saying.
pub fn render_cold_start_banner(report: &UpdateReport) -> Option<String> {
    if !report.update_available {
        return None;
    }
    Some(match &report.tombstone_message {
        Some(message) => format!("{message}\n{}", available_line(report)),
        None => available_line(report),
    })
}

/// The manual check's answer — both outcomes are reported, unlike the
/// quiet cold start (spec: a manual row in Settings → About).
pub fn render_manual_result(report: &UpdateReport) -> String {
    if !report.update_available {
        return format!("Up to date ({}).", report.current_version);
    }
    render_cold_start_banner(report).expect("available reports render")
}

fn available_line(report: &UpdateReport) -> String {
    format!(
        "Update available: {} — install it via the Homebrew Channel.",
        report.latest_version
    )
}

/// Where the released manifest lives once the packaging pipeline (#34)
/// publishes it: the repo's GitHub release feed, HBC manifest shape.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/acheronian/webos-bluepad/releases/latest/download/manifest.json";

/// What can go wrong with a courtesy look at the release manifest.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("the update source could not be reached: {0}")]
    Fetch(String),
    #[error("the manifest is not valid: {0}")]
    Parse(String),
    #[error("the tombstone manifest names no successor channel")]
    NoSuccessor,
    #[error("the tombstone chain never reached a release manifest")]
    TombstoneLoop,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn parses_a_release_manifest() {
        // The HBC manifest shape (repository.json's manifestUrl target):
        // an app id, a version, and packaging fields we don't act on.
        let manifest = ReleaseManifest::parse(
            r#"{"id":"com.acheronian.bluepad","version":"0.2.0","ipkUrl":"https://example/bluepad.ipk"}"#,
        )
        .expect("valid manifest");
        assert_eq!(manifest.id.as_deref(), Some("com.acheronian.bluepad"));
        assert_eq!(manifest.version.as_deref(), Some("0.2.0"));
        assert!(!manifest.discontinued);
        assert_eq!(manifest.successor, None);
        assert_eq!(manifest.message, None);
    }

    #[test]
    fn parses_a_tombstone_manifest() {
        // The ticket's shape: discontinued + successor manifest URL + message.
        let manifest = ReleaseManifest::parse(
            r#"{"discontinued":true,"successor":"https://new-host.example/manifest.json","message":"This channel has moved."}"#,
        )
        .expect("valid tombstone");
        assert!(manifest.discontinued);
        assert_eq!(
            manifest.successor.as_deref(),
            Some("https://new-host.example/manifest.json")
        );
        assert_eq!(manifest.message.as_deref(), Some("This channel has moved."));
        assert_eq!(manifest.version, None);
    }

    #[test]
    fn rejects_a_manifest_that_is_not_json() {
        let error = ReleaseManifest::parse("<html>404</html>").expect_err("not json");
        assert!(error.to_string().contains("not valid"));
    }

    /// A fetcher stub: URLs to canned bodies or errors, and a log of the
    /// URLs asked for (the injected seam — no network in unit tests).
    struct FakeFetcher<'a> {
        pages: &'a [(&'a str, Result<&'a str, &'a str>)],
        asked: RefCell<Vec<String>>,
    }

    impl<'a> FakeFetcher<'a> {
        fn new(pages: &'a [(&'a str, Result<&'a str, &'a str>)]) -> Self {
            FakeFetcher {
                pages,
                asked: RefCell::new(vec![]),
            }
        }
    }

    impl ManifestFetcher for FakeFetcher<'_> {
        fn fetch(&self, url: &str) -> Result<String, UpdateError> {
            self.asked.borrow_mut().push(url.to_string());
            match self.pages.iter().find(|(known, _)| *known == url) {
                Some((_, Ok(body))) => Ok((*body).to_string()),
                Some((_, Err(message))) => Err(UpdateError::Fetch((*message).to_string())),
                None => Err(UpdateError::Fetch(format!("no canned page for {url}"))),
            }
        }
    }

    const FEED: &str = "https://example.example/manifest.json";

    #[test]
    fn check_reports_an_available_update() {
        let pages = [(
            FEED,
            Ok(r#"{"id":"com.acheronian.bluepad","version":"0.2.0"}"#),
        )];
        let fetcher = FakeFetcher::new(&pages);
        let report = check_update(&fetcher, FEED, "0.1.0").expect("check");
        assert!(report.update_available);
        assert_eq!(report.latest_version, "0.2.0");
        assert_eq!(report.current_version, "0.1.0");
        assert_eq!(report.manifest_url, FEED);
        assert_eq!(report.tombstone_message, None);
    }

    #[test]
    fn check_follows_a_tombstone_to_its_successor_and_surfaces_the_message() {
        const NEW_FEED: &str = "https://new-host.example/manifest.json";
        let pages = [
            (
                FEED,
                Ok(r#"{"discontinued":true,"successor":"https://new-host.example/manifest.json","message":"This channel has moved."}"#),
            ),
            (
                NEW_FEED,
                Ok(r#"{"id":"com.acheronian.bluepad","version":"0.3.0"}"#),
            ),
        ];
        let fetcher = FakeFetcher::new(&pages);
        let report = check_update(&fetcher, FEED, "0.1.0").expect("check");
        // The check reads the successor, not the tombstone, for the version.
        assert_eq!(report.latest_version, "0.3.0");
        assert_eq!(report.manifest_url, NEW_FEED);
        assert!(report.update_available);
        assert_eq!(report.tombstone_message.as_deref(), Some("This channel has moved."));
        assert_eq!(*fetcher.asked.borrow(), vec![FEED, NEW_FEED]);
    }

    #[test]
    fn a_tombstone_without_a_successor_is_an_error() {
        let pages = [(FEED, Ok(r#"{"discontinued":true,"message":"gone"}"#))];
        let fetcher = FakeFetcher::new(&pages);
        let error = check_update(&fetcher, FEED, "0.1.0").expect_err("no successor");
        assert!(matches!(error, UpdateError::NoSuccessor));
    }

    #[test]
    fn check_reports_up_to_date_when_the_manifest_matches_this_build() {
        let pages = [(
            FEED,
            Ok(r#"{"id":"com.acheronian.bluepad","version":"0.1.0"}"#),
        )];
        let fetcher = FakeFetcher::new(&pages);
        let report = check_update(&fetcher, FEED, "0.1.0").expect("check");
        assert!(!report.update_available);
    }

    #[test]
    fn check_propagates_fetch_failures() {
        let pages = [(FEED, Err("dns failure"))];
        let fetcher = FakeFetcher::new(&pages);
        let error = check_update(&fetcher, FEED, "0.1.0").expect_err("offline");
        assert!(matches!(error, UpdateError::Fetch(_)));
    }

    fn state_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "bluepad-ui-state-{name}-{}.json",
            std::process::id()
        ))
    }

    #[test]
    fn absent_ui_state_loads_defaults_and_the_first_check_is_due() {
        let store = UiStateStore::load("/nonexistent/bluepad/ui-state.json");
        assert_eq!(store.update_source(), None);
        assert_eq!(store.cached_update(), None);
        assert!(store.update_check_due(1_000));
    }

    #[test]
    fn update_source_overrides_the_manifest_url_and_round_trips() {
        let path = state_path("source");
        let mut store = UiStateStore::load(&path);
        assert_eq!(store.effective_manifest_url(), DEFAULT_MANIFEST_URL);
        store
            .set_update_source(Some("https://mirror.example/manifest.json".into()))
            .expect("save");

        let reloaded = UiStateStore::load(&path);
        assert_eq!(
            reloaded.update_source(),
            Some("https://mirror.example/manifest.json")
        );
        assert_eq!(reloaded.effective_manifest_url(), "https://mirror.example/manifest.json");
        assert!(reloaded.update_check_due(1_000));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn clearing_the_update_source_falls_back_to_the_default_feed() {
        let path = state_path("clear");
        let mut store = UiStateStore::load(&path);
        store.set_update_source(Some("https://mirror.example/manifest.json".into())).unwrap();
        store.set_update_source(None).unwrap();
        assert_eq!(UiStateStore::load(&path).effective_manifest_url(), DEFAULT_MANIFEST_URL);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn the_cold_start_check_runs_at_most_once_per_24h_and_caches_its_result() {
        let path = state_path("throttle");
        let mut store = UiStateStore::load(&path);
        assert!(store.update_check_due(1_000));

        let report = UpdateReport {
            current_version: "0.1.0".into(),
            latest_version: "0.2.0".into(),
            update_available: true,
            manifest_url: FEED.into(),
            tombstone_message: None,
        };
        store.record_update_check(report.clone(), 1_000).expect("save");

        // Inside the 24 h window: not due, and the cached result is served.
        assert!(!store.update_check_due(1_000 + UPDATE_CHECK_INTERVAL_SECS - 1));
        let reloaded = UiStateStore::load(&path);
        assert_eq!(reloaded.cached_update(), Some(&report));
        assert!(!reloaded.update_check_due(1_000 + UPDATE_CHECK_INTERVAL_SECS - 1));
        // The window has passed: due again.
        assert!(reloaded.update_check_due(1_000 + UPDATE_CHECK_INTERVAL_SECS));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unparseable_ui_state_loads_defaults_and_is_kept_as_bad() {
        let path = state_path("bad");
        std::fs::write(&path, "{ not json ").unwrap();
        let store = UiStateStore::load(&path);
        assert_eq!(store.update_source(), None);
        assert!(store.update_check_due(1_000));
        assert!(path.with_extension("json.bad").exists());
        let _ = std::fs::remove_file(path.with_extension("json.bad"));
    }

    fn report(available: bool, tombstone: Option<&str>) -> UpdateReport {
        UpdateReport {
            current_version: "0.1.0".into(),
            latest_version: "0.2.0".into(),
            update_available: available,
            manifest_url: FEED.into(),
            tombstone_message: tombstone.map(Into::into),
        }
    }

    #[test]
    fn cold_start_shows_a_banner_only_when_an_update_is_available() {
        // Silence for the up-to-date case: a courtesy check that found
        // nothing must not clutter the main view.
        assert_eq!(render_cold_start_banner(&report(false, None)), None);
        assert_eq!(
            render_cold_start_banner(&report(true, None)).as_deref(),
            Some("Update available: 0.2.0 — install it via the Homebrew Channel.")
        );
        // A followed tombstone's message rides along.
        assert_eq!(
            render_cold_start_banner(&report(true, Some("This channel has moved."))).as_deref(),
            Some(
                "This channel has moved.\nUpdate available: 0.2.0 — install it via the Homebrew Channel."
            )
        );
    }

    #[test]
    fn the_manual_check_reports_both_outcomes() {
        assert_eq!(
            render_manual_result(&report(false, None)).as_str(),
            "Up to date (0.1.0)."
        );
        assert_eq!(
            render_manual_result(&report(true, None)).as_str(),
            "Update available: 0.2.0 — install it via the Homebrew Channel."
        );
        assert_eq!(
            render_manual_result(&report(true, Some("This channel has moved."))).as_str(),
            "This channel has moved.\nUpdate available: 0.2.0 — install it via the Homebrew Channel."
        );
    }

    #[test]
    fn cold_start_checks_when_due_and_records_the_attempt() {
        let path = state_path("cold-ok");
        let pages = [(
            FEED,
            Ok(r#"{"id":"com.acheronian.bluepad","version":"0.2.0"}"#),
        )];
        let fetcher = FakeFetcher::new(&pages);
        let mut store = UiStateStore::load(&path);
        store.set_update_source(Some(FEED.into())).expect("save");

        let banner = cold_start_check(&mut store, &fetcher, 1_000, "0.1.0");
        assert_eq!(
            banner.as_deref(),
            Some("Update available: 0.2.0 — install it via the Homebrew Channel.")
        );
        // The attempt is stamped: not due again inside the window.
        assert!(!store.update_check_due(1_001));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cold_start_is_silent_when_the_check_fails_but_still_throttles() {
        let path = state_path("cold-offline");
        let pages = [(FEED, Err("dns failure"))];
        let fetcher = FakeFetcher::new(&pages);
        let mut store = UiStateStore::load(&path);
        store.set_update_source(Some(FEED.into())).expect("save");

        // Offline failures are silent on cold start (spec: Update behavior).
        assert_eq!(cold_start_check(&mut store, &fetcher, 1_000, "0.1.0"), None);
        // The attempt still counts against the 24 h throttle.
        assert!(!store.update_check_due(1_001));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cold_start_inside_the_window_serves_the_cached_result_without_fetching() {
        let path = state_path("cold-cached");
        let pages: [(&str, Result<&str, &str>); 0] = [];
        let fetcher = FakeFetcher::new(&pages);
        let mut store = UiStateStore::load(&path);
        store.set_update_source(Some(FEED.into())).expect("save");
        store
            .record_update_check(report(true, None), 1_000)
            .expect("save");

        let banner = cold_start_check(&mut store, &fetcher, 1_000 + 60, "0.1.0");
        assert_eq!(
            banner.as_deref(),
            Some("Update available: 0.2.0 — install it via the Homebrew Channel.")
        );
        assert!(fetcher.asked.borrow().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_future_schema_version_is_rejected_not_obeyed() {
        let path = state_path("future");
        std::fs::write(
            &path,
            r#"{"schema_version":99,"update_source":"https://evil.example/manifest.json"}"#,
        )
        .unwrap();
        let store = UiStateStore::load(&path);
        assert_eq!(store.update_source(), None);
        assert!(path.with_extension("json.bad").exists());
        let _ = std::fs::remove_file(path.with_extension("json.bad"));
    }

    #[test]
    fn newer_version_is_detected_across_segments() {
        // Expected values from the spec's versioning: plain dotted numbers.
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.10.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
