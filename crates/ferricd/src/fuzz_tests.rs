//! Robustness tests: parsers that read attacker-controlled bytes (manifests, headers,
//! URLs, file names, feeds) must return an error or a value, never panic. Inputs are
//! deterministic pseudo-random mutations of realistic seeds plus raw noise.

use crate::{chunk, media, rss_watch, security};
use std::path::Path;

struct Xorshift(u64);

impl Xorshift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `count` inputs: noise and byte-level mutations (flip, delete, duplicate) of the seeds.
pub(crate) fn inputs(seeds: &[&[u8]], count: usize) -> Vec<Vec<u8>> {
    let mut rng = Xorshift(0x9E37_79B9_7F4A_7C15);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut v = if i % 4 == 0 || seeds.is_empty() {
            (0..(rng.next() % 300) as usize)
                .map(|_| rng.next() as u8)
                .collect()
        } else {
            seeds[(rng.next() as usize) % seeds.len()].to_vec()
        };
        for _ in 0..(rng.next() % 6) {
            if v.is_empty() {
                break;
            }
            let at = (rng.next() as usize) % v.len();
            match rng.next() % 3 {
                0 => v[at] = rng.next() as u8,
                1 => {
                    v.remove(at);
                }
                _ => {
                    let b = v[at];
                    v.insert(at, b);
                }
            }
        }
        out.push(v);
    }
    out
}

const HLS: &[u8] = b"#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"a\",NAME=\"en\",URI=\"a.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080\nhi/index.m3u8\n";
const DASH: &[u8] = b"<MPD><Period><AdaptationSet><BaseURL>v/</BaseURL><Representation id=\"v\" bandwidth=\"800000\" width=\"640\" height=\"360\"/></AdaptationSet></Period></MPD>";
const RSS: &[u8] = b"<rss><channel><item><enclosure url=\"http://e.example/a.zip\"/><link>http://e.example/b</link></item></channel></rss>";

#[test]
fn hls_and_dash_parsers_never_panic() {
    for data in inputs(&[HLS], 3000) {
        let _ = media::parse_hls_variants("https://e.example/m.m3u8", &data);
        let _ = media::parse_hls_alternatives("https://e.example/m.m3u8", &data);
    }
    for data in inputs(&[DASH], 3000) {
        let text = String::from_utf8_lossy(&data);
        let _ = media::parse_dash_all("https://e.example/m.mpd", &text);
    }
}

#[test]
fn header_url_and_filename_handling_never_panic() {
    let seeds: [&[u8]; 4] = [
        b"attachment; filename=\"a.zip\"",
        b"attachment; filename*=UTF-8''%E2%82%AC.bin",
        b"https://user:pw@host.example:8080/a/../b?x=1#f",
        b"../..\\evil\x00.exe",
    ];
    for data in inputs(&seeds, 6000) {
        let text = String::from_utf8_lossy(&data).to_string();
        let _ = chunk::filename_from_content_disposition(Some(&text));
        let _ = security::safe_filename(&text, Path::new("."));
        let _ = security::validate_save_path(&text);
    }
    // validate_url resolves host names (real DNS), so it gets fewer rounds.
    for data in inputs(&seeds, 150) {
        let _ = security::validate_url(&String::from_utf8_lossy(&data));
    }
}

#[test]
fn rss_link_extraction_never_panics() {
    for data in inputs(&[RSS], 3000) {
        let _ = rss_watch::extract_links(&String::from_utf8_lossy(&data));
    }
}
