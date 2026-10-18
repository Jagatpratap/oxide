//! The release list (`releases.json` on the repository's main branch, written by
//! `scripts/update_releases.py` when a release is published).

use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Where the Hub reads the release list. `$OXIDE_RELEASES_URL` overrides it (a URL, or a
/// local file path for testing).
pub const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/Jagatpratap/oxide/main/releases.json";

/// The platform this Hub was built for (`aarch64-apple-darwin`, ...), which picks the download.
pub const TARGET: &str = env!("OXIDE_HUB_TARGET");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Production,
    Beta,
    Alpha,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Production, Channel::Beta, Channel::Alpha];

    pub fn label(self) -> &'static str {
        match self {
            Channel::Production => "Production",
            Channel::Beta => "Beta",
            Channel::Alpha => "Alpha",
        }
    }

    /// The channel a version string implies: `-alpha...` alpha, any other pre-release
    /// (`-beta`, `-rc`) beta, none production. Matches `scripts/update_releases.py`.
    pub fn of_version(version: &str) -> Channel {
        match version.split_once('-') {
            None => Channel::Production,
            Some((_, pre)) if pre.to_ascii_lowercase().starts_with("alpha") => Channel::Alpha,
            Some(_) => Channel::Beta,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Download {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Release {
    pub version: String,
    pub channel: Channel,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub notes: String,
    /// Set by hand on a broken version: still listed for projects that use it, not offered.
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub downloads: BTreeMap<String, Download>,
}

impl Release {
    /// The download for this computer.
    pub fn download(&self) -> Option<&Download> {
        self.downloads.get(TARGET)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HubRelease {
    pub version: String,
    #[serde(default)]
    pub downloads: BTreeMap<String, Download>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub schema: u32,
    #[serde(default)]
    pub hub: Option<HubRelease>,
    #[serde(default)]
    pub releases: Vec<Release>,
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut m: Manifest =
            serde_json::from_str(text).map_err(|e| format!("bad release list: {e}"))?;
        if m.schema > 1 {
            return Err(
                "the release list is newer than this Hub understands: download the latest Hub"
                    .into(),
            );
        }
        m.releases
            .sort_by(|a, b| compare_versions(&b.version, &a.version));
        Ok(m)
    }

    pub fn find(&self, version: &str) -> Option<&Release> {
        self.releases.iter().find(|r| r.version == version)
    }

    /// Releases in `channel` that have a download for this computer, newest first.
    pub fn offered(&self, channel: Channel) -> impl Iterator<Item = &Release> {
        self.releases
            .iter()
            .filter(move |r| r.channel == channel && !r.deprecated && r.download().is_some())
    }

    /// A newer Hub than this one, if the list has one for this computer.
    pub fn hub_update(&self) -> Option<&HubRelease> {
        self.hub.as_ref().filter(|h| {
            h.downloads.contains_key(TARGET)
                && compare_versions(&h.version, env!("CARGO_PKG_VERSION")) == Ordering::Greater
        })
    }
}

/// Reads the release list from `source` (an http(s) URL or a file path).
pub fn fetch(source: &str) -> Result<Manifest, String> {
    let text = if source.starts_with("http://") || source.starts_with("https://") {
        crate::net::agent()
            .get(source)
            .call()
            .map_err(|e| format!("cannot download the release list: {e}"))?
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("cannot read the release list: {e}"))?
    } else {
        std::fs::read_to_string(source).map_err(|e| format!("{source}: {e}"))?
    };
    Manifest::parse(&text)
}

pub fn source() -> String {
    std::env::var("OXIDE_RELEASES_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned())
}

/// Semantic-version order: `0.2.0 > 0.2.0-rc.1 > 0.2.0-beta.2 > 0.2.0-beta.1 > 0.1.9`.
/// A leading `v` is ignored; a version that isn't numeric sorts by text.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let split = |v: &str| {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = v.split_once('-').unwrap_or((v, ""));
        let nums: Vec<u64> = core.split('.').map(|n| n.parse().unwrap_or(0)).collect();
        (nums, pre.to_owned())
    };
    let ((na, pa), (nb, pb)) = (split(a), split(b));
    let len = na.len().max(nb.len());
    for i in 0..len {
        let (x, y) = (
            na.get(i).copied().unwrap_or(0),
            nb.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return x.cmp(&y);
        }
    }
    match (pa.is_empty(), pb.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let (ia, ib): (Vec<&str>, Vec<&str>) = (
                pa.split(['.', '-']).collect(),
                pb.split(['.', '-']).collect(),
            );
            for (x, y) in ia.iter().zip(&ib) {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            ia.len().cmp(&ib.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_like_semver() {
        let mut v = vec![
            "0.1.9",
            "0.2.0-beta.1",
            "0.2.0",
            "0.2.0-rc.1",
            "0.2.0-alpha.3",
            "0.2.0-beta.10",
            "0.2.0-beta.2",
            "0.10.0",
        ];
        v.sort_by(|a, b| compare_versions(b, a));
        assert_eq!(
            v,
            [
                "0.10.0",
                "0.2.0",
                "0.2.0-rc.1",
                "0.2.0-beta.10",
                "0.2.0-beta.2",
                "0.2.0-beta.1",
                "0.2.0-alpha.3",
                "0.1.9"
            ]
        );
        assert_eq!(compare_versions("v1.0.0", "1.0"), Ordering::Equal);
    }

    #[test]
    fn channels_from_versions() {
        assert_eq!(Channel::of_version("1.0.0"), Channel::Production);
        assert_eq!(Channel::of_version("1.0.0-beta.1"), Channel::Beta);
        assert_eq!(Channel::of_version("1.0.0-rc.2"), Channel::Beta);
        assert_eq!(Channel::of_version("1.0.0-alpha"), Channel::Alpha);
    }

    #[test]
    fn manifest_parses_and_filters_for_this_computer() {
        let text = format!(
            r#"{{
              "schema": 1,
              "hub": {{ "version": "99.0.0", "downloads": {{ "{TARGET}": {{ "url": "u", "sha256": "s" }} }} }},
              "releases": [
                {{ "version": "0.1.0", "channel": "production",
                   "downloads": {{ "{TARGET}": {{ "url": "a", "sha256": "1", "size": 5 }} }} }},
                {{ "version": "0.2.0-beta.1", "channel": "beta",
                   "downloads": {{ "{TARGET}": {{ "url": "b", "sha256": "2" }} }} }},
                {{ "version": "0.2.0-beta.2", "channel": "beta", "deprecated": true,
                   "downloads": {{ "{TARGET}": {{ "url": "c", "sha256": "3" }} }} }},
                {{ "version": "0.3.0-alpha.1", "channel": "alpha",
                   "downloads": {{ "some-other-target": {{ "url": "d", "sha256": "4" }} }} }}
              ]
            }}"#
        );
        let m = Manifest::parse(&text).unwrap();
        assert_eq!(m.releases[0].version, "0.3.0-alpha.1", "newest first");
        let beta: Vec<_> = m
            .offered(Channel::Beta)
            .map(|r| r.version.as_str())
            .collect();
        assert_eq!(beta, ["0.2.0-beta.1"], "deprecated versions aren't offered");
        assert_eq!(
            m.offered(Channel::Alpha).count(),
            0,
            "no download for this computer"
        );
        assert_eq!(m.find("0.1.0").unwrap().download().unwrap().size, 5);
        assert!(m.hub_update().is_some());
        assert!(Manifest::parse(r#"{"schema": 2}"#).is_err());
        // The repository's own file must always parse.
        let repo = include_str!("../../../releases.json");
        Manifest::parse(repo).unwrap();
    }
}
