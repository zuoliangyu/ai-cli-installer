use futures_util::stream::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

use crate::error::{AppError, Result};
use crate::upstream::{self, Manifest};
use crate::validate;

/// Single per-mirror upper bound. Reused by every step that races mirrors:
/// `fetch_version`, `fetch_manifest`, and `downloader::send_with_timeout`
/// (the "got response headers" guard around binary downloads).
///
/// 8s is long enough for a slow-but-alive mirror over a flaky link, short
/// enough that a dead mirror (typically `official` from CN) doesn't drag
/// the whole install into reqwest's 60s global timeout.
pub const PER_MIRROR_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Mirror {
    /// Mirror that exposes the same path layout as `downloads.claude.ai/claude-code-releases`.
    /// `base` should NOT have a trailing slash.
    Upstream { name: String, base: String },

    /// GitHub Release-based mirror. Optionally fronted by a GH proxy host.
    /// Releases must be tagged `v{VERSION}` and contain:
    ///   - manifest.json
    ///   - {platform}-{binary}   (e.g. win32-x64-claude.exe, linux-x64-claude)
    ///   - latest.txt / stable.txt (channel pointer files)
    GhRelease {
        name: String,
        owner: String,
        repo: String,
        proxy: Option<String>,
    },
}

impl Mirror {
    pub fn name(&self) -> &str {
        match self {
            Mirror::Upstream { name, .. } => name,
            Mirror::GhRelease { name, .. } => name,
        }
    }

    /// Whether this mirror talks to the origin directly (no third-party GH
    /// proxy in between). Only a trusted mirror may single-handedly supply a
    /// manifest — see [`fetch_json_trusted`].
    pub fn is_trusted(&self) -> bool {
        match self {
            Mirror::Upstream { base, .. } => base.starts_with("https://downloads.claude.ai/"),
            Mirror::GhRelease { proxy, .. } => proxy.is_none(),
        }
    }

    /// Prefix a raw GitHub URL with the proxy host (if any). Shared by every
    /// GhRelease URL builder below.
    fn via_proxy(proxy: Option<&str>, raw: String) -> String {
        match proxy {
            Some(p) => format!("{}/{}", p.trim_end_matches('/'), raw),
            None => raw,
        }
    }

    /// `https://github.com/{owner}/{repo}/releases/download/v{version}/{asset}`,
    /// proxied when configured. `None` for Upstream mirrors.
    fn release_asset(&self, version: &str, asset: &str) -> Option<String> {
        match self {
            Mirror::Upstream { .. } => None,
            Mirror::GhRelease {
                owner, repo, proxy, ..
            } => Some(Self::via_proxy(
                proxy.as_deref(),
                format!(
                    "https://github.com/{}/{}/releases/download/v{}/{}",
                    owner, repo, version, asset
                ),
            )),
        }
    }

    pub fn version_url(&self, channel: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => format!("{}/{}", base, channel),
            Mirror::GhRelease {
                owner, repo, proxy, ..
            } => Self::via_proxy(
                proxy.as_deref(),
                format!(
                    "https://raw.githubusercontent.com/{}/{}/main/channels/{}.txt",
                    owner, repo, channel
                ),
            ),
        }
    }

    pub fn manifest_url(&self, version: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => format!("{}/{}/manifest.json", base, version),
            Mirror::GhRelease { .. } => self.asset_url(version, "manifest.json"),
        }
    }

    pub fn binary_url(&self, version: &str, platform: &str, binary: &str) -> String {
        match self {
            Mirror::Upstream { base, .. } => {
                format!("{}/{}/{}/{}", base, version, platform, binary)
            }
            // GH Release assets are flat — encode platform into asset name
            Mirror::GhRelease { .. } => {
                self.asset_url(version, &format!("{}-{}", platform, binary))
            }
        }
    }

    /// Generic GH release asset URL (for arbitrary asset names like .tgz files
    /// that don't follow the `{platform}-{binary}` template). Only meaningful
    /// for `GhRelease` mirrors — Upstream returns an empty string (caller
    /// should filter to GhRelease mirrors when downloading npm tarballs).
    pub fn asset_url(&self, version: &str, asset: &str) -> String {
        self.release_asset(version, asset).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MirrorList {
    pub mirrors: Vec<Mirror>,
}

impl MirrorList {
    /// Built-in fallback for the Claude Code mirror — kept for backwards compat /
    /// default UI display. Tools should call `builtin_for` with their own repo.
    pub fn builtin() -> Self {
        Self::builtin_for("claude-code-mirror", /* with_upstream */ true)
    }

    /// Built-in fallback parameterized by mirror repo name.
    /// `with_upstream`: include downloads.claude.ai (only valid for claude-code).
    pub fn builtin_for(repo: &str, with_upstream: bool) -> Self {
        let owner = "zuoliangyu";

        let gh = |name: &str, proxy: Option<&str>| Mirror::GhRelease {
            name: name.to_string(),
            owner: owner.to_string(),
            repo: repo.to_string(),
            proxy: proxy.map(String::from),
        };

        let mut mirrors = Vec::new();
        if with_upstream {
            mirrors.push(Mirror::Upstream {
                name: "official".to_string(),
                base: "https://downloads.claude.ai/claude-code-releases".to_string(),
            });
        }
        mirrors.extend([
            gh("github-direct", None),
            gh("gh-proxy", Some("https://gh-proxy.com")),
            gh("fastgit", Some("https://fastgit.cc")),
            gh("yylx", Some("https://git.yylx.win")),
            gh("chenc", Some("https://github.chenc.dev")),
            gh("ghproxy-net", Some("https://ghproxy.net")),
            gh("ghfast", Some("https://ghfast.top")),
        ]);
        Self { mirrors }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MirrorProbe {
    pub name: String,
    pub ok: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

/// Probe each mirror with a one-byte ranged GET on the version endpoint and
/// return latency.
///
/// Pre-v0.5.4 this used HEAD, but several free GH proxies answer HEAD with
/// 403/405 while serving GET fine, so working mirrors showed up as dead in
/// the sidebar. `Range: bytes=0-0` keeps the transfer at one byte where it's
/// honored; the pointer files are only a few bytes where it isn't.
pub async fn probe_all(client: &reqwest::Client, list: &MirrorList) -> Vec<MirrorProbe> {
    let probes = list.mirrors.iter().map(|m| {
        let client = client.clone();
        let url = m.version_url("latest");
        let name = m.name().to_string();
        async move {
            let start = Instant::now();
            let req = client
                .get(&url)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send();
            match tokio::time::timeout(Duration::from_secs(5), req).await {
                Ok(Ok(r)) if r.status().is_success() => MirrorProbe {
                    name,
                    ok: true,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error: None,
                },
                Ok(Ok(r)) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some(format!("status {}", r.status())),
                },
                Ok(Err(e)) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some(e.to_string()),
                },
                Err(_) => MirrorProbe {
                    name,
                    ok: false,
                    latency_ms: None,
                    error: Some("timeout".to_string()),
                },
            }
        }
    });
    futures_util::future::join_all(probes).await
}

/// Race every mirror in parallel for the version string, returning whichever
/// responds first with a well-formed version.
///
/// History: we used to walk mirrors sequentially, which meant a single stalled
/// mirror at the head of the list (typically `official` when `downloads.claude.ai`
/// is unreachable from CN) burned the entire outer 10s budget in
/// `fetch_channel_version` before any working mirror got a chance — visible to
/// the user as "获取版本失败" even when the sidebar probe showed 7/8 mirrors up,
/// because probe is parallel with its own per-mirror timeout. Parallel GETs
/// here mirror that probe shape so the two stay consistent.
///
/// Each per-mirror attempt is capped at 8s. The whole race finishes when the
/// first Ok arrives, so a hot mirror returning in ~300ms means total latency is
/// bounded by that mirror, not by the slowest peer.
///
/// A rate-limited proxy may answer `200 OK` with an HTML error page; bodies
/// that don't pass [`validate::is_valid_version`] count as that mirror
/// failing, so they can neither win the race nor reach the version cache.
pub async fn fetch_version<'a>(
    client: &reqwest::Client,
    list: &'a MirrorList,
    channel: &str,
) -> Result<(&'a Mirror, String)> {
    let mut tasks: futures_util::stream::FuturesUnordered<_> = list
        .mirrors
        .iter()
        .map(|m| {
            let url = m.version_url(channel);
            let client = client.clone();
            let mirror_name = m.name().to_string();
            async move {
                let r =
                    tokio::time::timeout(PER_MIRROR_TIMEOUT, upstream::fetch_text(&client, &url))
                        .await;
                match r {
                    Ok(Ok(v)) if validate::is_valid_version(&v) => Ok((m, v)),
                    Ok(Ok(v)) if v.is_empty() => Err((mirror_name, "empty body".to_string())),
                    Ok(Ok(v)) => Err((
                        mirror_name,
                        format!("invalid version body: {}", validate::truncate_for_display(&v)),
                    )),
                    Ok(Err(e)) => Err((mirror_name, e.to_string())),
                    Err(_) => Err((mirror_name, format!("{}s timeout", PER_MIRROR_TIMEOUT.as_secs()))),
                }
            }
        })
        .collect();

    let mut failures: Vec<(String, String)> = Vec::new();
    while let Some(res) = tasks.next().await {
        match res {
            Ok(pair) => return Ok(pair),
            Err(fail) => failures.push(fail),
        }
    }
    for (name, err) in &failures {
        tracing::warn!("fetch_version mirror {}: {}", name, err);
    }
    Err(AppError::AllMirrorsFailed)
}

/// Fetch the manifest of a given version, racing every mirror in parallel.
///
/// The manifest carries the SHA256 we verify binaries against, and the
/// binaries come from the same proxy pool — so a manifest served by one
/// third-party proxy is not trustworthy on its own. See
/// [`fetch_json_trusted`] for the acceptance rule.
///
/// Pre-v0.5 this was a sequential `for` loop with no per-mirror timeout, so
/// a single dead mirror at the head of the list (typically `official` when
/// downloads.claude.ai is unreachable from CN) would burn reqwest's 60s
/// global timeout before the next mirror got a chance.
pub async fn fetch_manifest<'a>(
    client: &reqwest::Client,
    list: &'a MirrorList,
    version: &str,
) -> Result<(&'a Mirror, Manifest)> {
    fetch_json_trusted(
        client,
        list,
        "manifest.json",
        |m| Some(m.manifest_url(version)),
        manifest_fingerprint,
    )
    .await
}

/// Per-platform fingerprint of a native manifest: everything that decides
/// which bytes we accept (file names, checksum, size).
type ManifestFingerprint = Vec<(String, String, Option<String>, String, u64)>;

fn manifest_fingerprint(m: &Manifest) -> Option<ManifestFingerprint> {
    if m.platforms.is_empty() {
        return None;
    }
    Some(
        m.platforms
            .iter()
            .map(|(plat, e)| {
                (
                    plat.clone(),
                    e.binary.clone(),
                    e.runtime_binary.clone(),
                    e.checksum.to_ascii_lowercase(),
                    e.size,
                )
            })
            .collect(),
    )
}

/// Race every mirror for a JSON metadata document under a trust policy:
///
/// 1. A response from a **trusted** mirror (official / github-direct, see
///    [`Mirror::is_trusted`]) wins immediately.
/// 2. Only once every trusted mirror has failed (common from CN), a
///    proxy-served document is accepted if **at least two different
///    proxies** returned the same `fingerprint`. A single malicious proxy
///    therefore can't swap both the checksum and the binary.
///
/// All requests start at once, so in the CN case (github.com times out) the
/// extra latency over "first proxy wins" is bounded by `PER_MIRROR_TIMEOUT`.
/// `url_for` returning `None` skips a mirror (e.g. Upstream for npm assets).
pub async fn fetch_json_trusted<'a, T, K, U, F>(
    client: &reqwest::Client,
    list: &'a MirrorList,
    what: &str,
    url_for: U,
    fingerprint: F,
) -> Result<(&'a Mirror, T)>
where
    T: DeserializeOwned,
    K: PartialEq,
    U: Fn(&Mirror) -> Option<String>,
    F: Fn(&T) -> Option<K>,
{
    let mut trusted_pending = 0usize;
    let mut tasks: futures_util::stream::FuturesUnordered<_> = list
        .mirrors
        .iter()
        .filter_map(|m| {
            let url = url_for(m).filter(|u| !u.is_empty())?;
            let trusted = m.is_trusted();
            if trusted {
                trusted_pending += 1;
            }
            let client = client.clone();
            Some(async move {
                let r = tokio::time::timeout(
                    PER_MIRROR_TIMEOUT,
                    upstream::fetch_json::<T>(&client, &url),
                )
                .await;
                let r = match r {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e.to_string()),
                    Err(_) => Err(format!("{}s timeout", PER_MIRROR_TIMEOUT.as_secs())),
                };
                (m, trusted, r)
            })
        })
        .collect();

    let mut proxied: Vec<(&'a Mirror, T, Option<K>)> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();
    while let Some((m, trusted, res)) = tasks.next().await {
        match res {
            Ok(doc) if trusted => {
                tracing::info!("{} from trusted mirror {}", what, m.name());
                return Ok((m, doc));
            }
            Ok(doc) => {
                let fp = fingerprint(&doc);
                proxied.push((m, doc, fp));
            }
            Err(e) => {
                failures.push((m.name().to_string(), e));
                if trusted {
                    trusted_pending -= 1;
                }
            }
        }
        if trusted_pending == 0 {
            let fps: Vec<(&str, Option<&K>)> = proxied
                .iter()
                .map(|(m, _, fp)| (m.name(), fp.as_ref()))
                .collect();
            if let Some(idx) = consensus_index(&fps) {
                let (m, doc, _) = proxied.swap_remove(idx);
                tracing::info!(
                    "{} from proxy {} (confirmed by another proxy; trusted mirrors unreachable)",
                    what,
                    m.name()
                );
                return Ok((m, doc));
            }
        }
    }

    for (name, err) in &failures {
        tracing::warn!("fetch {} mirror {}: {}", what, name, err);
    }
    if proxied.is_empty() {
        return Err(AppError::AllMirrorsFailed);
    }
    let names: Vec<&str> = proxied.iter().map(|(m, _, _)| m.name()).collect();
    tracing::warn!(
        "{}: trusted mirrors failed and proxies {:?} did not agree",
        what,
        names
    );
    Err(AppError::Other(format!(
        "无法安全获取 {}：官方源 / GitHub 直连均不可用，且没有两个代理返回一致的校验信息\
         （收到 {} 份）。为防止安装被篡改的文件已中止，请稍后重试或更换网络。",
        what,
        proxied.len()
    )))
}

/// Index of the first entry whose fingerprint is shared by an entry from a
/// *different* mirror. Entries without a fingerprint never count.
fn consensus_index<K: PartialEq>(fps: &[(&str, Option<&K>)]) -> Option<usize> {
    fps.iter().enumerate().find_map(|(i, (name_i, fp_i))| {
        let fp_i = (*fp_i)?;
        fps.iter()
            .enumerate()
            .any(|(j, (name_j, fp_j))| j != i && name_i != name_j && *fp_j == Some(fp_i))
            .then_some(i)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gh(name: &str, proxy: Option<&str>) -> Mirror {
        Mirror::GhRelease {
            name: name.into(),
            owner: "o".into(),
            repo: "r".into(),
            proxy: proxy.map(String::from),
        }
    }

    #[test]
    fn proxy_prefix_is_applied_once() {
        let m = gh("p", Some("https://proxy.example/"));
        assert_eq!(
            m.asset_url("1.2.3", "a.tgz"),
            "https://proxy.example/https://github.com/o/r/releases/download/v1.2.3/a.tgz"
        );
        assert_eq!(
            m.binary_url("1.2.3", "linux-x64", "claude"),
            "https://proxy.example/https://github.com/o/r/releases/download/v1.2.3/linux-x64-claude"
        );
        assert_eq!(
            gh("d", None).version_url("latest"),
            "https://raw.githubusercontent.com/o/r/main/channels/latest.txt"
        );
        let up = Mirror::Upstream {
            name: "u".into(),
            base: "https://downloads.claude.ai/x".into(),
        };
        assert_eq!(up.asset_url("1.2.3", "a.tgz"), "");
        assert_eq!(up.manifest_url("1.2.3"), "https://downloads.claude.ai/x/1.2.3/manifest.json");
    }

    #[test]
    fn only_direct_mirrors_are_trusted() {
        let list = MirrorList::builtin();
        let trusted: Vec<&str> = list
            .mirrors
            .iter()
            .filter(|m| m.is_trusted())
            .map(|m| m.name())
            .collect();
        assert_eq!(trusted, vec!["official", "github-direct"]);
    }

    fn manifest_response(checksum: &str) -> Vec<u8> {
        let body = format!(
            r#"{{"version":"1.0.0","platforms":{{"linux-x64":{{"binary":"claude","checksum":"{}","size":1}}}}}}"#,
            checksum
        );
        crate::test_support::response(
            "200 OK",
            &[format!("Content-Length: {}", body.len())],
            body.as_bytes(),
        )
    }

    async fn proxy_serving(name: &str, checksum: &'static str) -> Mirror {
        let url = crate::test_support::serve(move |_, _| manifest_response(checksum)).await;
        gh(name, Some(url.trim_end_matches("/file")))
    }

    #[tokio::test]
    async fn proxy_manifest_needs_two_agreeing_proxies() {
        let client = reqwest::Client::new();
        let fetch = |mirrors: Vec<Mirror>| {
            let client = client.clone();
            async move {
                let list = MirrorList { mirrors };
                fetch_manifest(&client, &list, "1.0.0")
                    .await
                    .map(|(m, man)| (m.name().to_string(), man.platforms["linux-x64"].checksum.clone()))
            }
        };

        // A single proxy is never enough.
        assert!(fetch(vec![proxy_serving("p1", "aa").await]).await.is_err());
        // Two proxies disagreeing: refuse.
        assert!(fetch(vec![proxy_serving("p1", "aa").await, proxy_serving("p2", "bb").await])
            .await
            .is_err());
        // Two of three agree: accepted, and it's the agreed-upon checksum.
        let (_, checksum) = fetch(vec![
            proxy_serving("p1", "aa").await,
            proxy_serving("evil", "bb").await,
            proxy_serving("p3", "aa").await,
        ])
        .await
        .unwrap();
        assert_eq!(checksum, "aa");
    }

    #[test]
    fn consensus_needs_two_distinct_mirrors() {
        let a = 1;
        let b = 2;
        assert_eq!(consensus_index(&[("p1", Some(&a))]), None);
        assert_eq!(consensus_index(&[("p1", Some(&a)), ("p2", Some(&b))]), None);
        assert_eq!(consensus_index(&[("p1", Some(&a)), ("p1", Some(&a))]), None);
        assert_eq!(consensus_index::<i32>(&[("p1", None), ("p2", None)]), None);
        assert_eq!(
            consensus_index(&[("p1", Some(&b)), ("p2", Some(&a)), ("p3", Some(&a))]),
            Some(1)
        );
    }
}
