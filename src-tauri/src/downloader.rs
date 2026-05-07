use futures_util::StreamExt;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::error::Result;

#[derive(Clone, Serialize)]
pub struct DownloadProgress {
    pub tool_id: String,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub mirror: String,
}

pub async fn download_to_file(
    client: &reqwest::Client,
    app: &AppHandle,
    tool_id: &str,
    mirror_name: &str,
    url: &str,
    dest: &Path,
) -> Result<u64> {
    let resp = client.get(url).send().await?.error_for_status()?;
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
            let _ = app.emit(
                "download-progress",
                DownloadProgress {
                    tool_id: tool_id.to_string(),
                    downloaded,
                    total,
                    mirror: mirror_name.to_string(),
                },
            );
            last_emit = std::time::Instant::now();
        }
    }
    file.flush().await?;

    let _ = app.emit(
        "download-progress",
        DownloadProgress {
            tool_id: tool_id.to_string(),
            downloaded,
            total,
            mirror: mirror_name.to_string(),
        },
    );

    Ok(downloaded)
}
