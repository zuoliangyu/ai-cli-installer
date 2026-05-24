use futures_util::StreamExt;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::error::{AppError, Result};
use crate::mirrors::PER_MIRROR_TIMEOUT;
use crate::progress::{DownloadProgress, ProgressCallback};

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

pub async fn download_to_file(
    client: &reqwest::Client,
    progress: &ProgressCallback,
    tool_id: &str,
    mirror_name: &str,
    url: &str,
    dest: &Path,
) -> Result<u64> {
    let resp = send_with_timeout(client, url).await?;
    let total = resp.content_length();

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = File::create(dest).await?;
    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
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
    file.flush().await?;

    progress(DownloadProgress {
        tool_id: tool_id.to_string(),
        downloaded,
        total,
        mirror: mirror_name.to_string(),
    });

    Ok(downloaded)
}
