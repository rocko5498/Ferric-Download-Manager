//! Task 153 — thin CLI against local ferricd API.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ferric_common::API_LOCK_FILE;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ferric", about = "Ferric download manager CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Daemon health
    Health,
    /// List downloads
    List,
    /// Add a URL
    Add {
        url: String,
        #[arg(long)]
        filename: Option<String>,
    },
    /// Privacy dashboard summary
    Privacy,
    /// Export audit JSON to stdout
    AuditExport,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let (base, token) = read_endpoint().await?;
    let client = reqwest::Client::new();

    match cli.cmd {
        Cmd::Health => {
            let v: serde_json::Value = client
                .get(format!("{base}/health"))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::List => {
            let v: serde_json::Value = client
                .get(format!("{base}/downloads"))
                .header("X-Ferric-Auth", &token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::Add { url, filename } => {
            let body = serde_json::json!({ "url": url, "filename": filename });
            let v: serde_json::Value = client
                .post(format!("{base}/downloads"))
                .header("X-Ferric-Auth", &token)
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::Privacy => {
            let v: serde_json::Value = client
                .get(format!("{base}/privacy/dashboard"))
                .header("X-Ferric-Auth", &token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        Cmd::AuditExport => {
            let v: serde_json::Value = client
                .get(format!("{base}/audit/export"))
                .header("X-Ferric-Auth", &token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
    }
    Ok(())
}

async fn read_endpoint() -> Result<(String, String)> {
    let path = data_dir()?.join(API_LOCK_FILE);
    let contents = tokio::fs::read_to_string(&path)
        .await
        .with_context(|| format!("read {}; is ferricd running?", path.display()))?;
    let mut lines = contents.lines();
    let port: u16 = lines
        .next()
        .and_then(|l| l.trim().parse().ok())
        .ok_or_else(|| anyhow::anyhow!("bad lock port"))?;
    let token = lines
        .next()
        .map(|l| l.trim().to_string())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow::anyhow!("bad lock token"))?;
    Ok((format!("http://127.0.0.1:{port}"), token))
}

fn data_dir() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from(
        ferric_common::APP_QUALIFIER,
        ferric_common::APP_ORGANIZATION,
        ferric_common::APP_NAME,
    )
    .ok_or_else(|| anyhow::anyhow!("data dir"))?;
    Ok(dirs.data_local_dir().to_path_buf())
}
