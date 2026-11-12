//! What the Rust CLI and daemon share beneath their transports: the
//! capability tables, the build's identity, the status payload, and the
//! narrow store interface the shell calls (plan 21, R1).

pub mod capabilities;
#[cfg(feature = "fake")]
pub mod fake;
pub mod status;
pub mod store;

/// The running binary's identity: `version_text()` and the version in
/// `daemon.ready` and `status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildInfo {
    /// The product version (`kassiber.__version__`).
    pub version: &'static str,
    /// The packaging channel, when a release job embedded one.
    pub channel: Option<&'static str>,
    /// The source commit, when a release job embedded one.
    pub commit: Option<&'static str>,
}

impl BuildInfo {
    /// `version_text()`: `Kassiber <version>`, then `(channel, commit
    /// <12 hex>)` for packaged builds.
    pub fn version_text(&self) -> String {
        let mut details = Vec::new();
        if let Some(channel) = self.channel.map(str::trim).filter(|c| !c.is_empty()) {
            details.push(channel.to_owned());
        }
        if let Some(commit) = self.commit.map(str::trim).filter(|c| !c.is_empty()) {
            details.push(format!(
                "commit {}",
                commit.chars().take(12).collect::<String>()
            ));
        }
        if details.is_empty() {
            format!("Kassiber {}", self.version)
        } else {
            format!("Kassiber {} ({})", self.version, details.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_text_matches_build_info() {
        let mut info = BuildInfo {
            version: "0.22.78",
            channel: None,
            commit: None,
        };
        assert_eq!(info.version_text(), "Kassiber 0.22.78");
        info.channel = Some("stable");
        info.commit = Some("0123456789abcdef");
        assert_eq!(
            info.version_text(),
            "Kassiber 0.22.78 (stable, commit 0123456789ab)"
        );
        info.channel = Some(" ");
        assert_eq!(
            info.version_text(),
            "Kassiber 0.22.78 (commit 0123456789ab)"
        );
    }
}
