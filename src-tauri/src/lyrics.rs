use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LyricsPayload {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    pub plain: Option<String>,
    pub source: String,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct LrcLibItem {
    #[serde(rename = "trackName")]
    pub track_name: Option<String>,
    #[serde(rename = "artistName")]
    pub artist_name: Option<String>,
    pub duration: Option<f64>,
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    pub instrumental: Option<bool>,
}

static LYRICS_CACHE: OnceLock<Mutex<HashMap<String, LyricsPayload>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<String, LyricsPayload>> {
    LYRICS_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(title: &str, artist: &str) -> String {
    format!("{}|{}", artist.trim().to_lowercase(), title.trim().to_lowercase())
}

fn sanitize_filename(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn lyrics_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?.join("lyrics_cache");
    let _ = fs::create_dir_all(&dir);
    Some(dir)
}

pub fn parse_lrc(lrc_text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();

    for raw_line in lrc_text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Handle potential multiple timestamps per line: [00:10.00][00:20.00] Repeated line
        let mut rest = trimmed;
        let mut timestamps = Vec::new();

        while rest.starts_with('[') {
            if let Some(close_idx) = rest.find(']') {
                let tag = &rest[1..close_idx];
                if let Some(ms) = parse_timestamp(tag) {
                    timestamps.push(ms);
                }
                rest = &rest[close_idx + 1..];
            } else {
                break;
            }
        }

        let text = rest.trim().to_string();
        for ts in timestamps {
            lines.push(LyricLine {
                time_ms: ts,
                text: text.clone(),
            });
        }
    }

    lines.sort_by_key(|l| l.time_ms);
    lines
}

fn parse_timestamp(tag: &str) -> Option<u64> {
    // Expected formats: mm:ss.xx, mm:ss.xxx, mm:ss
    let parts: Vec<&str> = tag.split(':').collect();
    if parts.len() != 2 {
        return None;
    }

    let minutes: u64 = parts[0].parse().ok()?;
    let sec_parts: Vec<&str> = parts[1].split('.').collect();

    let seconds: u64 = sec_parts[0].parse().ok()?;
    let millis: u64 = if sec_parts.len() > 1 {
        let ms_str = sec_parts[1];
        if ms_str.len() == 1 {
            ms_str.parse::<u64>().ok()? * 100
        } else if ms_str.len() == 2 {
            ms_str.parse::<u64>().ok()? * 10
        } else if ms_str.len() >= 3 {
            ms_str[..3].parse::<u64>().ok()?
        } else {
            0
        }
    } else {
        0
    };

    Some(minutes * 60_000 + seconds * 1_000 + millis)
}

fn is_noise_tag(inside: &str) -> bool {
    let lower = inside.to_lowercase();
    lower.starts_with("feat")
        || lower.starts_with("ft.")
        || lower.starts_with("ft ")
        || lower.starts_with("with ")
        || lower.starts_with("prod")
        || lower.contains("remaster")
        || lower.contains("live")
        || lower.contains("version")
        || lower.contains("deluxe")
        || lower.contains("edition")
        || lower.contains("official")
        || lower.contains("audio")
        || lower.contains("video")
        || lower.contains("lyric")
        || lower.contains("sped up")
        || lower.contains("slowed")
        || lower.contains("acoustic")
        || lower.contains("explicit")
        || lower.contains("bonus")
        || lower.contains("radio edit")
}

fn clean_track_title(title: &str) -> String {
    let mut t = title.trim().to_string();

    loop {
        let prev = t.clone();

        // Strip trailing brackets [...]
        if let Some(start) = t.rfind('[') {
            if let Some(end) = t.rfind(']') {
                if end > start && end == t.len() - 1 {
                    t = t[..start].trim().to_string();
                }
            }
        }

        // Strip trailing parentheses containing noise
        if let Some(start) = t.rfind('(') {
            if let Some(end) = t.rfind(')') {
                if end > start && end == t.len() - 1 {
                    let inside = &t[start + 1..end];
                    if is_noise_tag(inside) {
                        t = t[..start].trim().to_string();
                    }
                }
            }
        }

        // Strip common trailing suffixes separated by " - "
        if let Some(idx) = t.rfind(" - ") {
            let suffix = &t[idx + 3..];
            if is_noise_tag(suffix)
                || suffix.to_lowercase().contains("mix")
                || suffix.to_lowercase().contains("edit")
                || suffix.to_lowercase().contains("mono")
                || suffix.to_lowercase().contains("çizik")
            {
                t = t[..idx].trim().to_string();
            }
        }

        if t == prev {
            break;
        }
    }

    t.trim().to_string()
}

fn clean_primary_artist(artist: &str) -> String {
    let trimmed = artist.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut result = trimmed;
    for sep in [
        ", ", " & ", " feat.", " feat ", " Feat.", " Feat ", " FEAT.", " ft.", " ft ", " Ft.",
        " Ft ", " x ", " X ",
    ] {
        if let Some(idx) = result.find(sep) {
            if idx > 0 {
                result = result[..idx].trim();
            }
        }
    }

    result.to_string()
}

fn has_usable_lyrics(item: &LrcLibItem) -> bool {
    let has_synced = item.synced_lyrics.as_ref().is_some_and(|s| !s.trim().is_empty());
    let has_plain = item.plain_lyrics.as_ref().is_some_and(|s| !s.trim().is_empty());
    has_synced || has_plain
}

async fn try_get_lrclib(
    client: &reqwest::Client,
    track: &str,
    artist: &str,
    dur_secs: Option<u64>,
) -> Option<LrcLibItem> {
    if track.trim().is_empty() {
        return None;
    }

    let mut url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}",
        urlencoding::encode(track),
        urlencoding::encode(artist)
    );
    if let Some(d) = dur_secs {
        url.push_str(&format!("&duration={}", d));
    }

    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    let item = serde_json::from_str::<LrcLibItem>(&text).ok()?;
    if has_usable_lyrics(&item) {
        Some(item)
    } else {
        None
    }
}

fn pick_best_search_item(
    items: Vec<LrcLibItem>,
    target_title: &str,
    target_artist: &str,
    dur_secs: Option<u64>,
) -> Option<LrcLibItem> {
    let norm_title = clean_track_title(target_title).to_lowercase();
    let norm_artist = clean_primary_artist(target_artist).to_lowercase();

    items
        .into_iter()
        .filter(has_usable_lyrics)
        .max_by_key(|item| {
            let mut score: i64 = 0;

            if item.synced_lyrics.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                score += 1000;
            }

            if let Some(ref item_title) = item.track_name {
                let it = clean_track_title(item_title).to_lowercase();
                if !norm_title.is_empty() {
                    if it == norm_title {
                        score += 200;
                    } else if it.contains(&norm_title) || norm_title.contains(&it) {
                        score += 80;
                    }
                }
            }

            if let Some(ref item_artist) = item.artist_name {
                let ia = item_artist.trim().to_lowercase();
                if !norm_artist.is_empty() {
                    if ia == norm_artist {
                        score += 200;
                    } else if ia.contains(&norm_artist) || norm_artist.contains(&ia) {
                        score += 100;
                    }
                }
            }

            if let (Some(target_dur), Some(item_dur)) = (dur_secs, item.duration) {
                let diff = (target_dur as f64 - item_dur).abs();
                if diff <= 2.0 {
                    score += 120;
                } else if diff <= 6.0 {
                    score += 70;
                } else if diff <= 15.0 {
                    score += 30;
                } else {
                    score -= ((diff - 15.0) as i64).min(120);
                }
            }

            score
        })
}

async fn try_search_lrclib(
    client: &reqwest::Client,
    url: &str,
    target_title: &str,
    target_artist: &str,
    dur_secs: Option<u64>,
) -> Option<LrcLibItem> {
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    let items = serde_json::from_str::<Vec<LrcLibItem>>(&text).ok()?;
    pick_best_search_item(items, target_title, target_artist, dur_secs)
}

pub async fn fetch_lyrics(
    app: &AppHandle,
    raw_title: &str,
    raw_artist: &str,
    duration_ms: Option<u64>,
) -> Result<Option<LyricsPayload>, String> {
    let title = raw_title.trim();
    let artist = raw_artist.trim();

    if title.is_empty() {
        return Ok(None);
    }

    let key = cache_key(title, artist);

    // 1. Check in-memory cache
    if let Ok(cache) = get_cache().lock() {
        if let Some(cached) = cache.get(&key) {
            return Ok(Some(cached.clone()));
        }
    }

    // 2. Check disk cache
    if let Some(cache_dir) = lyrics_cache_dir(app) {
        let file_path = cache_dir.join(format!("{}.json", sanitize_filename(&key)));
        if file_path.exists() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                if let Ok(payload) = serde_json::from_str::<LyricsPayload>(&content) {
                    if let Ok(mut cache) = get_cache().lock() {
                        cache.insert(key.clone(), payload.clone());
                    }
                    return Ok(Some(payload));
                }
            }
        }
    }

    // 3. Query LRCLIB via reqwest
    let client = reqwest::Client::builder()
        .user_agent("Fern/1.4.0 (https://github.com/kagangungor/Fern)")
        .timeout(Duration::from_secs(6))
        .build()
        .map_err(|e| e.to_string())?;

    let cleaned_title = clean_track_title(title);
    let primary_artist = clean_primary_artist(artist);
    let dur_secs = duration_ms.filter(|&d| d > 0).map(|d| d / 1000);

    // Attempt 1: Exact match with duration
    let mut item_opt = try_get_lrclib(&client, title, artist, dur_secs).await;

    // Attempt 2: Exact match without duration (handles ±2s+ differences between Spotify and LRCLIB)
    if item_opt.is_none() && dur_secs.is_some() {
        item_opt = try_get_lrclib(&client, title, artist, None).await;
    }

    // Attempt 3: Cleaned title and primary artist without duration
    if item_opt.is_none() && (cleaned_title != title || primary_artist != artist) {
        item_opt = try_get_lrclib(&client, &cleaned_title, &primary_artist, None).await;
    }

    // Attempt 4: Structured search with track_name and artist_name
    if item_opt.is_none() && !primary_artist.is_empty() {
        let search_url = format!(
            "https://lrclib.net/api/search?track_name={}&artist_name={}",
            urlencoding::encode(&cleaned_title),
            urlencoding::encode(&primary_artist)
        );
        item_opt = try_search_lrclib(&client, &search_url, title, artist, dur_secs).await;
    }

    // Attempt 5: Free-text search fallback (now supports both synced and plain lyrics)
    if item_opt.is_none() {
        let search_query = if primary_artist.is_empty() {
            cleaned_title.clone()
        } else {
            format!("{} {}", primary_artist, cleaned_title)
        };
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(&search_query)
        );
        item_opt = try_search_lrclib(&client, &search_url, title, artist, dur_secs).await;
    }

    let payload = if let Some(item) = item_opt {
        let plain_non_empty = item.plain_lyrics.filter(|p| !p.trim().is_empty());

        if let Some(synced) = item.synced_lyrics.filter(|s| !s.trim().is_empty()) {
            let lines = parse_lrc(&synced);
            if !lines.is_empty() {
                Some(LyricsPayload {
                    synced: true,
                    lines,
                    plain: plain_non_empty,
                    source: "LRCLIB".to_string(),
                })
            } else if let Some(plain) = plain_non_empty {
                Some(LyricsPayload {
                    synced: false,
                    lines: Vec::new(),
                    plain: Some(plain),
                    source: "LRCLIB".to_string(),
                })
            } else {
                None
            }
        } else if let Some(plain) = plain_non_empty {
            Some(LyricsPayload {
                synced: false,
                lines: Vec::new(),
                plain: Some(plain),
                source: "LRCLIB".to_string(),
            })
        } else {
            None
        }
    } else {
        None
    };

    // Cache only successful results in memory and on disk so transient failures can retry
    if let Some(ref p) = payload {
        if let Ok(mut cache) = get_cache().lock() {
            cache.insert(key.clone(), p.clone());
        }
        if let Some(cache_dir) = lyrics_cache_dir(app) {
            let file_path = cache_dir.join(format!("{}.json", sanitize_filename(&key)));
            if let Ok(json) = serde_json::to_string(p) {
                let _ = fs::write(file_path, json);
            }
        }
    }

    Ok(payload)
}

// Module for simple urlencoding without extra dependencies
mod urlencoding {
    pub fn encode(s: &str) -> String {
        let mut encoded = String::new();
        for byte in s.bytes() {
            match byte {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    encoded.push(byte as char);
                }
                b' ' => encoded.push('+'),
                _ => {
                    encoded.push_str(&format!("%{:02X}", byte));
                }
            }
        }
        encoded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(
        track: &str,
        artist: &str,
        duration: Option<f64>,
        synced: Option<&str>,
        plain: Option<&str>,
    ) -> LrcLibItem {
        LrcLibItem {
            track_name: Some(track.to_string()),
            artist_name: Some(artist.to_string()),
            duration,
            synced_lyrics: synced.map(|s| s.to_string()),
            plain_lyrics: plain.map(|s| s.to_string()),
            instrumental: Some(false),
        }
    }

    #[test]
    fn cleans_titles_and_unicode_artists_without_panicking() {
        assert_eq!(clean_track_title("Yağsın Dünya Üstüme"), "Yağsın Dünya Üstüme");
        assert_eq!(clean_track_title("Lanet (feat. Khontkar) [Official Audio]"), "Lanet");
        assert_eq!(clean_track_title("Düşler Sokağı - Remastered 2021"), "Düşler Sokağı");

        assert_eq!(clean_primary_artist("İbrahim Tatlıses, Müslüm Gürses"), "İbrahim Tatlıses");
        assert_eq!(clean_primary_artist("Keişan & Ben Fero"), "Keişan");
        assert_eq!(clean_primary_artist("Wegh"), "Wegh");
    }

    #[test]
    fn search_picker_falls_back_to_plain_lyrics_when_synced_missing() {
        let items = vec![
            make_item("Yağsın Dünya Üstüme", "Wegh", Some(154.0), None, Some("sözler uzak")),
            make_item("Yağsın Dünya Üstüme", "Wegh", Some(137.0), None, Some("sözler yakın")),
        ];
        let picked = pick_best_search_item(items, "Yağsın Dünya Üstüme", "Wegh", Some(132))
            .expect("should pick plain lyrics item when synced is missing");
        assert_eq!(picked.duration, Some(137.0));
        assert_eq!(picked.plain_lyrics.as_deref(), Some("sözler yakın"));
    }

    #[test]
    fn search_picker_prefers_synced_over_plain() {
        let items = vec![
            make_item("Lanet", "Keişan", Some(267.0), None, Some("plain only")),
            make_item("Lanet", "Keişan", Some(267.0), Some("[00:26.87] Üstümüzde kara bulutlar"), Some("plain")),
        ];
        let picked = pick_best_search_item(items, "Lanet", "Keişan", Some(267))
            .expect("should pick synced lyrics item");
        assert!(picked.synced_lyrics.is_some());
    }

    #[test]
    fn parses_lrc_lines_and_sorts() {
        let lines = parse_lrc("[00:20.50]Second\n[00:10.00]First");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].time_ms, 10_000);
        assert_eq!(lines[0].text, "First");
        assert_eq!(lines[1].time_ms, 20_500);
        assert_eq!(lines[1].text, "Second");
    }
}
