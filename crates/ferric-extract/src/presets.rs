//! Built-in format preset library (Task 115).

/// Named yt-dlp-style format specs users can pick without learning the grammar.
#[derive(Debug, Clone, Copy)]
pub struct FormatPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub spec: &'static str,
}

pub const PRESETS: &[FormatPreset] = &[
    FormatPreset {
        id: "best",
        label: "Best video",
        spec: "bestvideo+bestaudio/best",
    },
    FormatPreset {
        id: "720p_mp4",
        label: "720p MP4",
        spec: "bestvideo[height<=720][ext=mp4]+bestaudio[ext=m4a]/best[height<=720]/best",
    },
    FormatPreset {
        id: "1080p",
        label: "1080p",
        spec: "bestvideo[height<=1080]+bestaudio/best[height<=1080]/best",
    },
    FormatPreset {
        id: "audio",
        label: "Audio only",
        spec: "bestaudio/best",
    },
    FormatPreset {
        id: "smallest",
        label: "Smallest file",
        spec: "worst",
    },
];

#[must_use]
pub fn list() -> &'static [FormatPreset] {
    PRESETS
}

#[must_use]
pub fn resolve(id_or_spec: &str) -> &str {
    PRESETS
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(id_or_spec) || p.label.eq_ignore_ascii_case(id_or_spec))
        .map(|p| p.spec)
        .unwrap_or(id_or_spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_preset_ids() {
        assert_eq!(resolve("audio"), "bestaudio/best");
        assert_eq!(resolve("bestvideo+bestaudio"), "bestvideo+bestaudio");
    }
}
