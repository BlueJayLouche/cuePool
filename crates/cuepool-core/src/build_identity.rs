//! One offline source identity for the GUI, diagnostics, logs and API.

#[derive(Debug, serde::Serialize)]
pub struct BuildIdentity {
    pub version: &'static str,
    /// Preserves CUEPOOL_BUILD_ID's existing meaning; otherwise the short SHA.
    pub build_id: Option<&'static str>,
    /// Always the actual full source commit, never a packaging override.
    pub commit: Option<&'static str>,
    pub dirty: Option<bool>,
    pub source_fingerprint: Option<&'static str>,
    pub version_tag: Option<&'static str>,
    pub commits_since_tag: Option<u64>,
    pub shallow: bool,
    pub publication: &'static str,
    pub display: &'static str,
    pub changes_baseline: &'static str,
    pub changes: &'static str,
}

include!(concat!(env!("OUT_DIR"), "/build_identity.rs"));

pub fn build_identity() -> String {
    BUILD.display.to_owned()
}
