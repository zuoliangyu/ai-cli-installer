use futures_util::StreamExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, Result};
use crate::mirrors::PER_MIRROR_TIMEOUT;
use crate::progress::{DownloadProgress, ProgressCallback};
use crate::verifier;

/// How many times we'll (re)connect to the SAME mirror before giving up and
/// letting the caller fall through to the next mirror in the chain. Attempt 1
/// is the initial download; the rest are resume attempts after a mid-stream
/// drop.
///
/// Same-mirror retry exists because the free GH proxies in the mirror list
/// (gh-proxy.com, ghfast.top, …) routinely reset the connection near the *tail*
/// of a large binary. Pre-v0.5 a single such drop deleted the partial file and
/// jumped to a different mirror, restarting from byte 0 — the user-visible
/// "download was almost done, then it switched lines and started over".
const MAX_ATTEMPTS: usize = 4;

/// If the body stream produces no new bytes for this long, treat the connection
/// as dead and trigger a resume attempt. This is the body-phase counterpart to
/// `PER_MIRROR_TIMEOUT`, which only guards the *header* phase: once headers
/// arrive the body stream is otherwise untimed, so a mirror that accepts the
/// connection then silently stops sending (an overloaded proxy that neither
/// closes nor feeds) would hang forever without this.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Write ceiling when the caller doesn't know the expected size. The largest
/// real asset (Claude Code native binary / Codex npm tarball) is ~250 MB;
/// this only exists so a hostile mirror can't stream until the disk is full.
pub const MAX_UNSIZED_DOWNLOAD: u64 = 1024 * 1024 * 1024;

/// Issue `GET url` and wait for response headers, bounded by
/// `PER_MIRROR_TIMEOUT`. The body stream is **not** under this timeout —
/// once headers arrive, a slow-but-alive download over flaky links is
/// expected, and we must not cut a partially-finished file off.
///
/// Pre-v0.5 callers used `client.get(url).send().await?` bare, which
/// meant a dead mirror would idle out on reqwest's 60s global timeout
/// instead of the 8s per-mirror budget. Every code path that walks a
/// mirror chain should go through this helper.
pub async fn send_with_timeout(
    client: &reqwest::Client,
    url: &str,
) -> Result<reqwest::Response> {
    let resp = tokio::time::timeout(PER_MIRROR_TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| {
            AppError::Other(format!(
                "{}s timeout waiting for response headers",
                PER_MIRROR_TIMEOUT.as_secs()
            ))
        })??;
    Ok(resp.error_for_status()?)
}

/// `dest` + `.part` — where bytes land until the download is complete.
fn part_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_os_string();
    p.push(".part");
    PathBuf::from(p)
}

/// Backwards-compatible entry point without a known size (capped at
/// [`MAX_UNSIZED_DOWNLOAD`]). Prefer [`download_to_file_limited`].
pub async fn download_to_file(
    client: &reqwest::Client,
    progress: &ProgressCallback,
    tool_id: &str,
    mirror_name: &str,
    url: &str,
    dest: &Path,
) -> Result<u64> {
    download_to_file_limited(client, progress, tool_id, mirror_name, url, dest, None).await
}

/// Download `url` into `dest`, with three layers of resilience over a single
/// mirror before the caller is expected to fall through to the next one:
///
/// 1. **Resume** — on a mid-stream drop we reopen the file in append mode and
///    re-request with `Range: bytes=N-`. A cooperating server replies `206
///    Partial Content` and we continue from byte N; a server that ignores the
///    header replies `200` and we transparently restart from byte 0.
/// 2. **Same-mirror retry** — up to `MAX_ATTEMPTS`, but only while we're still
///    making progress (see the resume guard below). A mirror that delivers 0
///    bytes is treated as dead and handed straight back to the caller.
/// 3. **Body idle timeout** — each chunk read is bounded by `BODY_IDLE_TIMEOUT`
///    so a silently-stalled connection is converted into a resume attempt
///    instead of hanging.
///
/// `expected_size` (from the manifest) bounds how much we accept: a mirror
/// sending more is cut off immediately, one ending early is resumed. `None`
/// (or 0) falls back to [`MAX_UNSIZED_DOWNLOAD`].
///
/// Bytes are written to `dest.part` and renamed onto `dest` only once the
/// transfer is complete, so `dest` never holds a truncated file. The `.part`
/// file is removed on failure.
///
/// Returns the total bytes written on success.
pub async fn download_to_file_limited(
    client: &reqwest::Client,
    progress: &ProgressCallback,
    tool_id: &str,
    mirror_name: &str,
    url: &str,
    dest: &Path,
    expected_size: Option<u64>,
) -> Result<u64> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = part_path(dest);
    let expected = expected_size.filter(|n| *n > 0);
    match download_attempts(client, progress, tool_id, mirror_name, url, &part, expected).await {
        Ok(n) => {
            tokio::fs::rename(&part, dest).await?;
            Ok(n)
        }
        Err(e) => {
            let _ = tokio::fs::remove_file(&part).await;
            Err(e)
        }
    }
}

async fn download_attempts(
    client: &reqwest::Client,
    progress: &ProgressCallback,
    tool_id: &str,
    mirror_name: &str,
    url: &str,
    part: &Path,
    expected: Option<u64>,
) -> Result<u64> {
    let cap = expected.unwrap_or(MAX_UNSIZED_DOWNLOAD);
    let mut downloaded: u64 = 0;
    let mut total: Option<u64> = expected;
    let mut last_err: Option<AppError> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        // On a resume attempt, ask the server to continue from where we stopped.
        let mut req = client.get(url);
        if downloaded > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", downloaded));
        }

        // Header phase is bounded by PER_MIRROR_TIMEOUT, same as send_with_timeout.
        let resp = match tokio::time::timeout(PER_MIRROR_TIMEOUT, req.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                // Connect-level failure: this mirror is unreachable, don't burn
                // retries on it — hand back to the caller's mirror chain.
                return Err(e.into());
            }
            Err(_) => {
                return Err(AppError::Other(format!(
                    "{}s timeout waiting for response headers",
                    PER_MIRROR_TIMEOUT.as_secs()
                )));
            }
        };

        // 416 on a resume: our Range starts at/after EOF. If we already hold
        // exactly the full size (manifest size, or `Content-Range: bytes */N`)
        // the file is complete — let the caller verify it instead of
        // throwing it away. Otherwise restart from 0.
        if downloaded > 0 && resp.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            if expected.or_else(|| unsatisfied_range_total(&resp)) == Some(downloaded) {
                return Ok(downloaded);
            }
            tracing::warn!(
                "{} answered 416 at {} bytes; restarting from 0 (attempt {}/{})",
                mirror_name,
                downloaded,
                attempt,
                MAX_ATTEMPTS
            );
            downloaded = 0;
            last_err = Some(AppError::Other(format!("{} 不支持断点续传（416）", mirror_name)));
            continue;
        }
        let resp = resp.error_for_status()?;

        // 206 means our Range was honored and we should append; anything else
        // (typically 200) means the server is resending from the start, so reset.
        let resuming = resp.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if downloaded > 0 && !resuming {
            downloaded = 0;
        }

        // Establish the full content size for progress. On a fresh 200 the
        // Content-Length IS the full size; on a 206 it's only the *remaining*
        // length, so the full size is already-downloaded + remaining. Keep the
        // first total we learn (the manifest size, when known, wins).
        let announced = match resp.content_length() {
            Some(len) if resuming => Some(downloaded + len),
            other => other,
        };
        if announced.is_some_and(|len| len > cap) {
            return Err(AppError::Other(format!(
                "{} 返回的文件大小（{} 字节）超出预期（{} 字节），已中止",
                mirror_name,
                announced.unwrap_or_default(),
                cap
            )));
        }
        if total.is_none() {
            total = announced;
        }

        // Append when resuming, truncate on a fresh start.
        let mut file = if downloaded > 0 {
            OpenOptions::new().append(true).open(part).await?
        } else {
            File::create(part).await?
        };

        let mut stream = resp.bytes_stream();
        let mut last_emit = std::time::Instant::now();
        let mut stream_err: Option<AppError> = None;

        loop {
            match tokio::time::timeout(BODY_IDLE_TIMEOUT, stream.next()).await {
                Ok(Some(Ok(bytes))) => {
                    // Hard limit: never write past the expected size. A mirror
                    // that over-delivers is broken or hostile; no resume.
                    if downloaded + bytes.len() as u64 > cap {
                        return Err(AppError::Other(format!(
                            "{} 发送的数据超过预期大小（{} 字节），已中止",
                            mirror_name, cap
                        )));
                    }
                    file.write_all(&bytes).await?;
                    downloaded += bytes.len() as u64;

                    // Throttle progress events to ~10/sec
                    if last_emit.elapsed().as_millis() >= 100 {
                        progress(DownloadProgress {
                            tool_id: tool_id.to_string(),
                            downloaded,
                            total,
                            mirror: mirror_name.to_string(),
                        });
                        last_emit = std::time::Instant::now();
                    }
                }
                Ok(Some(Err(e))) => {
                    stream_err = Some(e.into());
                    break;
                }
                Ok(None) => {
                    // Stream finished "cleanly" — but proxies sometimes close
                    // early without an error. Short = resume, not success.
                    if let Some(exp) = expected {
                        if downloaded < exp {
                            stream_err = Some(AppError::Other(format!(
                                "连接提前结束（{}/{} 字节）",
                                downloaded, exp
                            )));
                        }
                    }
                    break;
                }
                Err(_) => {
                    stream_err = Some(AppError::Other(format!(
                        "{}s idle timeout mid-download at {} bytes",
                        BODY_IDLE_TIMEOUT.as_secs(),
                        downloaded
                    )));
                    break;
                }
            }
        }
        file.flush().await?;
        drop(file);

        // An error right after the last byte still means we have the file.
        if stream_err.is_some() && expected == Some(downloaded) {
            stream_err = None;
        }

        match stream_err {
            None => {
                progress(DownloadProgress {
                    tool_id: tool_id.to_string(),
                    downloaded,
                    total,
                    mirror: mirror_name.to_string(),
                });
                return Ok(downloaded);
            }
            Some(e) => {
                tracing::warn!(
                    "download from {} interrupted at {} bytes (attempt {}/{}): {}",
                    mirror_name,
                    downloaded,
                    attempt,
                    MAX_ATTEMPTS,
                    e
                );
                last_err = Some(e);
                // Only resume if we actually made progress this round — a mirror
                // that dropped at 0 bytes won't get better by retrying, and we'd
                // rather spend that time on the next mirror in the chain.
                if attempt < MAX_ATTEMPTS && downloaded > 0 {
                    tokio::time::sleep(Duration::from_millis(300 * attempt as u64)).await;
                    continue;
                }
                break;
            }
        }
    }

    Err(last_err.unwrap_or(AppError::AllMirrorsFailed))
}

/// Full length from a 416's `Content-Range: bytes */N`.
fn unsatisfied_range_total(resp: &reqwest::Response) -> Option<u64> {
    resp.headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes */")?
        .trim()
        .parse()
        .ok()
}

/// Walk `candidates` (`(mirror name, url)`) in order until one yields a file
/// whose SHA256 matches `expected_sha256`. Shared by the Claude Code / Codex
/// native routes and the npm tarball route so all three behave the same:
///
/// - download errors **and** checksum mismatches both fall through to the
///   next mirror (a single tampered / truncated proxy must not abort the
///   install while healthy mirrors remain);
/// - the staged file is deleted after every failed candidate.
///
/// Returns the name of the mirror that delivered the verified file.
pub async fn download_verified(
    client: &reqwest::Client,
    progress: &ProgressCallback,
    tool_id: &str,
    candidates: Vec<(String, String)>,
    expected_sha256: &str,
    expected_size: Option<u64>,
    dest: &Path,
) -> Result<String> {
    let mut last_err: Option<AppError> = None;
    for (mirror, url) in candidates {
        tracing::info!("download attempt via {}: {}", mirror, url);
        let res = download_to_file_limited(
            client,
            progress,
            tool_id,
            &mirror,
            &url,
            dest,
            expected_size,
        )
        .await;
        let res = match res {
            Ok(_) => verifier::verify(dest, expected_sha256).await,
            Err(e) => Err(e),
        };
        match res {
            Ok(()) => return Ok(mirror),
            Err(e) => {
                tracing::warn!("mirror {} failed: {}", mirror, e);
                last_err = Some(e);
                let _ = tokio::fs::remove_file(dest).await;
            }
        }
    }
    Err(last_err.unwrap_or(AppError::AllMirrorsFailed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{response, serve};

    /// Chunked body without the terminating chunk: all bytes arrive, then
    /// the stream errors.
    fn chunked_then_drop(body: &[u8]) -> Vec<u8> {
        let mut out = response("200 OK", &["Transfer-Encoding: chunked".into()], b"");
        out.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\r\n");
        out
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ai-cli-installer-dl-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("file.bin")
    }

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn resumes_after_truncated_body() {
        let body = data(1000);
        let b = body.clone();
        let url = serve(move |idx, range| match (idx, range) {
            // Claims 1000 bytes, delivers 500, hangs up.
            (0, None) => response("200 OK", &["Content-Length: 1000".into()], &b[..500]),
            (_, Some(start)) => {
                let start = start as usize;
                response(
                    "206 Partial Content",
                    &[
                        format!("Content-Length: {}", b.len() - start),
                        format!("Content-Range: bytes {}-{}/{}", start, b.len() - 1, b.len()),
                    ],
                    &b[start..],
                )
            }
            _ => response("500 Internal Server Error", &[], b""),
        })
        .await;
        let dest = scratch("resume");
        let n = download_to_file_limited(
            &reqwest::Client::new(),
            &crate::progress::noop_progress(),
            "t",
            "m",
            &url,
            &dest,
            Some(1000),
        )
        .await
        .unwrap();
        assert_eq!(n, 1000);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn rejects_more_bytes_than_expected() {
        let url = serve(|_, _| chunked_then_drop(&data(4096))).await;
        let dest = scratch("oversize");
        let res = download_to_file_limited(
            &reqwest::Client::new(),
            &crate::progress::noop_progress(),
            "t",
            "m",
            &url,
            &dest,
            Some(1000),
        )
        .await;
        assert!(res.is_err());
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());

        let url = serve(|_, _| {
            response("200 OK", &["Content-Length: 4096".into()], &data(4096))
        })
        .await;
        let res = download_to_file_limited(
            &reqwest::Client::new(),
            &crate::progress::noop_progress(),
            "t",
            "m",
            &url,
            &dest,
            Some(1000),
        )
        .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn complete_file_survives_416_on_resume() {
        let body = data(1000);
        let b = body.clone();
        let url = serve(move |idx, _| {
            if idx == 0 {
                chunked_then_drop(&b)
            } else {
                response(
                    "416 Range Not Satisfiable",
                    &["Content-Range: bytes */1000".into(), "Content-Length: 0".into()],
                    b"",
                )
            }
        })
        .await;
        let dest = scratch("416");
        let n = download_to_file_limited(
            &reqwest::Client::new(),
            &crate::progress::noop_progress(),
            "t",
            "m",
            &url,
            &dest,
            None,
        )
        .await
        .unwrap();
        assert_eq!(n, 1000);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }

    #[tokio::test]
    async fn checksum_mismatch_falls_through_to_next_mirror() {
        use sha2::{Digest, Sha256};
        let good = data(300);
        let sha = hex::encode(Sha256::digest(&good));
        let g = good.clone();
        let bad_url = serve(|_, _| response("200 OK", &["Content-Length: 300".into()], &[0u8; 300])).await;
        let good_url = serve(move |_, _| response("200 OK", &["Content-Length: 300".into()], &g)).await;
        let dest = scratch("verified");
        let winner = download_verified(
            &reqwest::Client::new(),
            &crate::progress::noop_progress(),
            "t",
            vec![("bad".to_string(), bad_url), ("good".to_string(), good_url)],
            &sha,
            Some(300),
            &dest,
        )
        .await
        .unwrap();
        assert_eq!(winner, "good");
        assert_eq!(std::fs::read(&dest).unwrap(), good);
    }
}
