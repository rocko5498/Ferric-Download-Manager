//! Download categories the way IDM does them: the file type picks a category, each category has a
//! save folder (default `<downloads>/<Category>`, `General` = the downloads folder itself), and
//! the folder can be changed per category.

use std::path::{Path, PathBuf};

use sqlx::{Row, SqlitePool};

use crate::db;

pub const GENERAL: &str = "General";

/// Default categories and file types. `r0*` style entries match a prefix.
const BUILTIN: &[(&str, &str)] = &[
    (
        "Compressed",
        "zip rar r0* r1* arj gz sit sitx sea ace bz2 7z tar tgz xz zst",
    ),
    ("Documents", "doc pdf ppt pps docx pptx xls xlsx rtf epub"),
    ("Music", "mp3 wav wma mpa ram ra aac aif m4a flac opus tsa"),
    ("Programs", "exe msi msu msix apk"),
    (
        "Video",
        "avi mpg mpe mpeg asf wmv mov qt rm mp4 flv m4v webm ogv ogg mkv ts tsv 3gp",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub name: String,
    pub exts: Vec<String>,
    /// Folder under the downloads folder; `None` = the category name.
    pub subdir: Option<String>,
}

fn split_exts(list: &str) -> Vec<String> {
    list.split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .map(|e| {
            e.trim()
                .trim_start_matches("*.")
                .trim_start_matches('.')
                .to_ascii_lowercase()
        })
        .filter(|e| !e.is_empty())
        .collect()
}

/// Built-in categories plus the user's own (`download_categories` table), user ones first.
pub async fn load(pool: &SqlitePool) -> Vec<Category> {
    let mut cats: Vec<Category> = Vec::new();
    if let Ok(rows) = sqlx::query("SELECT name, pattern, save_subdir FROM download_categories")
        .fetch_all(pool)
        .await
    {
        for r in rows {
            let Ok(name) = r.try_get::<String, _>("name") else {
                continue;
            };
            let pattern: Option<String> = r.try_get("pattern").ok().flatten();
            let subdir: Option<String> = r.try_get("save_subdir").ok().flatten();
            cats.push(Category {
                name,
                exts: pattern.as_deref().map(split_exts).unwrap_or_default(),
                subdir: subdir.filter(|s| !s.trim().is_empty()),
            });
        }
    }
    for (name, exts) in BUILTIN {
        if !cats.iter().any(|c| c.name.eq_ignore_ascii_case(name)) {
            cats.push(Category {
                name: (*name).into(),
                exts: split_exts(exts),
                subdir: None,
            });
        }
    }
    cats
}

fn ext_matches(pattern: &str, ext: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(pattern == ext, |prefix| ext.starts_with(prefix))
}

/// File extension of the last path segment of `name` (a file name or a URL), lower-cased.
pub fn extension_of(name: &str) -> Option<String> {
    let name = name.split(['?', '#']).next().unwrap_or(name);
    let last = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let (_, ext) = last.rsplit_once('.')?;
    (!ext.is_empty() && ext.len() <= 8).then(|| ext.to_ascii_lowercase())
}

fn mime_category(mime: Option<&str>) -> Option<&'static str> {
    let m = mime?.split(';').next()?.trim().to_ascii_lowercase();
    match m.as_str() {
        m if m.starts_with("video/") => Some("Video"),
        m if m.starts_with("audio/") => Some("Music"),
        "application/pdf" | "application/msword" => Some("Documents"),
        m if m.contains("zip")
            || m.contains("tar")
            || m.contains("compressed")
            || m.contains("rar") =>
        {
            Some("Compressed")
        }
        m if m.contains("executable") || m.contains("x-msdownload") || m.contains("msi") => {
            Some("Programs")
        }
        _ => None,
    }
}

/// Category for a file: by extension first (like IDM), then by content type, else `General`.
pub fn classify(filename_or_url: &str, mime: Option<&str>, cats: &[Category]) -> String {
    if let Some(ext) = extension_of(filename_or_url) {
        if let Some(c) = cats
            .iter()
            .find(|c| c.exts.iter().any(|p| ext_matches(p, &ext)))
        {
            return c.name.clone();
        }
    }
    mime_category(mime).map_or_else(|| GENERAL.to_string(), str::to_string)
}

fn override_key(category: &str) -> String {
    format!("category_dir_{}", category.to_ascii_lowercase())
}

/// The downloads folder: the user's setting, else the OS downloads folder.
pub async fn base_dir(pool: &SqlitePool) -> PathBuf {
    if let Ok(Some(dir)) = db::settings::get(pool, "default_save_dir").await {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(ToOwned::to_owned))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}

/// Save folder for a category: the user's remembered folder, else `<base>/<subdir or name>`.
pub async fn folder_for(
    category: &str,
    base: &Path,
    cats: &[Category],
    pool: &SqlitePool,
) -> PathBuf {
    if category == GENERAL {
        if let Ok(Some(dir)) = db::settings::get(pool, &override_key(category)).await {
            if !dir.trim().is_empty() {
                return PathBuf::from(dir);
            }
        }
        return base.to_path_buf();
    }
    if let Ok(Some(dir)) = db::settings::get(pool, &override_key(category)).await {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let sub = cats
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(category))
        .and_then(|c| c.subdir.clone())
        .unwrap_or_else(|| category.to_string());
    // A subdir can never climb out of the downloads folder.
    let safe: PathBuf = Path::new(&sub)
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    base.join(if safe.as_os_str().is_empty() {
        Path::new(category)
    } else {
        safe.as_path()
    })
}

/// "Remember this path for the category": stored as a setting, absolute paths only.
pub async fn remember_folder(pool: &SqlitePool, category: &str, dir: &str) -> anyhow::Result<()> {
    crate::security::validate_save_path(dir)?;
    db::settings::set(pool, &override_key(category), dir).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin() -> Vec<Category> {
        BUILTIN
            .iter()
            .map(|(n, e)| Category {
                name: (*n).into(),
                exts: split_exts(e),
                subdir: None,
            })
            .collect()
    }

    #[test]
    fn file_type_picks_the_category_like_idm() {
        let c = builtin();
        assert_eq!(classify("Hello-World-master.zip", None, &c), "Compressed");
        assert_eq!(
            classify("https://x.test/a/setup.EXE?dl=1", None, &c),
            "Programs"
        );
        assert_eq!(classify("song.mp3", None, &c), "Music");
        assert_eq!(classify("movie.mkv", None, &c), "Video");
        assert_eq!(classify("paper.pdf", None, &c), "Documents");
        assert_eq!(classify("part.r07", None, &c), "Compressed", "r0* wildcard");
        assert_eq!(classify("notes.xyz", None, &c), "General");
    }

    #[test]
    fn content_type_is_the_fallback_when_the_name_says_nothing() {
        let c = builtin();
        assert_eq!(
            classify("download", Some("application/zip"), &c),
            "Compressed"
        );
        assert_eq!(
            classify("stream", Some("video/mp4; charset=x"), &c),
            "Video"
        );
        assert_eq!(
            classify("blob", Some("application/octet-stream"), &c),
            "General"
        );
    }

    #[test]
    fn user_categories_win_over_the_defaults() {
        let mut c = builtin();
        c.insert(
            0,
            Category {
                name: "Backups".into(),
                exts: split_exts("zip bak"),
                subdir: Some("Work/Backups".into()),
            },
        );
        assert_eq!(classify("db.zip", None, &c), "Backups");
    }

    #[test]
    fn extension_parsing_ignores_query_and_odd_names() {
        assert_eq!(
            extension_of("https://h/a/b.tar.gz?x=1.2"),
            Some("gz".into())
        );
        assert_eq!(extension_of("noext"), None);
        assert_eq!(extension_of("trailing."), None);
    }
}
