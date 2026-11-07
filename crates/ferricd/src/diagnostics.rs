use serde_json::json;
use std::path::PathBuf;

#[cfg(test)]
fn diagnostics_path() -> Option<PathBuf> {
    None
}

#[cfg(not(test))]
fn diagnostics_path() -> Option<PathBuf> {
    let data_dir = ferric_common::resolve_data_dir().ok()?;
    Some(data_dir.join("diagnostics").join("ferricd.jsonl"))
}

pub async fn event(name: &str, fields: serde_json::Value) {
    let Some(path) = diagnostics_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "event": name,
        "fields": fields,
    })
    .to_string();

    if let Ok(mut file) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        use tokio::io::AsyncWriteExt as _;
        let _ = file.write_all(line.as_bytes()).await;
        let _ = file.write_all(b"\n").await;
    }
}
