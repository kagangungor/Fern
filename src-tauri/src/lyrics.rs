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
    #[serde(rename = "plainLyrics")]
    pub plain_lyrics: Option<String>,
    #[serde(rename = "syncedLyrics")]
    pub synced_lyrics: Option<String>,
    pub instrumental: Option<bool>,
}

static LYRICS_CACHE: OnceLock<Mutex<HashMap<String, Option<LyricsPayload>>>> = OnceLock::new();

fn get_cache() -> &'static Mutex<HashMap<String, Option<LyricsPayload>>> {
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

fn clean_track_title(title: &str) -> String {
    let mut t = title.trim().to_string();

    // Strip common trailing suffixes separated by " - "
    if let Some(idx) = t.find(" - ") {
        let suffix = t[idx + 3..].to_lowercase();
        if suffix.contains("remaster")
            || suffix.contains("live")
            || suffix.contains("radio")
            || suffix.contains("edit")
            || suffix.contains("version")
            || suffix.contains("mix")
            || suffix.contains("bonus")
            || suffix.contains("deluxe")
            || suffix.contains("mono")
            || suffix.contains("lyric")
            || suffix.contains("çizik")
        {
            t = t[..idx].trim().to_string();
        }
    }

    // Strip parentheses containing noise
    if let Some(start) = t.rfind('(') {
        if let Some(end) = t.rfind(')') {
            if end > start {
                let inside = t[start + 1..end].to_lowercase();
                if inside.starts_with("feat")
                    || inside.starts_with("ft.")
                    || inside.starts_with("with")
                    || inside.contains("remaster")
                    || inside.contains("live")
                    || inside.contains("version")
                    || inside.contains("deluxe")
                    || inside.contains("edition")
                    || inside.contains("official")
                    || inside.contains("audio")
                    || inside.contains("video")
                    || inside.contains("lyric")
                {
                    t = format!("{}{}", &t[..start], &t[end + 1..]).trim().to_string();
                }
            }
        }
    }

    // Strip brackets [Official Video], [Remastered]
    if let Some(start) = t.rfind('[') {
        if let Some(end) = t.rfind(']') {
            if end > start {
                t = format!("{}{}", &t[..start], &t[end + 1..]).trim().to_string();
            }
        }
    }

    t.trim().to_string()
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
            return Ok(cached.clone());
        }
    }

    // 2. Check disk cache
    if let Some(cache_dir) = lyrics_cache_dir(app) {
        let file_path = cache_dir.join(format!("{}.json", sanitize_filename(&key)));
        if file_path.exists() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                if let Ok(payload) = serde_json::from_str::<LyricsPayload>(&content) {
                    if let Ok(mut cache) = get_cache().lock() {
                        cache.insert(key.clone(), Some(payload.clone()));
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
    let dur_secs = duration_ms.map(|d| d / 1000);

    // Attempt 1: Direct exact match
    let mut item_opt: Option<LrcLibItem> = None;

    let mut get_url = format!(
        "https://lrclib.net/api/get?track_name={}&artist_name={}",
        urlencoding::encode(title),
        urlencoding::encode(artist)
    );
    if let Some(d) = dur_secs {
        get_url.push_str(&format!("&duration={}", d));
    }

    if let Ok(resp) = client.get(&get_url).send().await {
        if resp.status().is_success() {
            if let Ok(text) = resp.text().await {
                if let Ok(item) = serde_json::from_str::<LrcLibItem>(&text) {
                    item_opt = Some(item);
                }
            }
        }
    }

    // Attempt 2: If cleaned title is different, try direct get with cleaned title
    if item_opt.is_none() && cleaned_title != title {
        let mut get_clean_url = format!(
            "https://lrclib.net/api/get?track_name={}&artist_name={}",
            urlencoding::encode(&cleaned_title),
            urlencoding::encode(artist)
        );
        if let Some(d) = dur_secs {
            get_clean_url.push_str(&format!("&duration={}", d));
        }

        if let Ok(resp) = client.get(&get_clean_url).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if let Ok(item) = serde_json::from_str::<LrcLibItem>(&text) {
                        item_opt = Some(item);
                    }
                }
            }
        }
    }

    // Attempt 3: Search fallback
    if item_opt.is_none() {
        let search_query = format!("{} {}", artist, cleaned_title);
        let search_url = format!(
            "https://lrclib.net/api/search?q={}",
            urlencoding::encode(&search_query)
        );

        if let Ok(resp) = client.get(&search_url).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if let Ok(items) = serde_json::from_str::<Vec<LrcLibItem>>(&text) {
                        // Prefer the first item that has synced lyrics
                        item_opt = items.into_iter().find(|i| {
                            i.synced_lyrics.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false)
                        });
                    }
                }
            }
        }
    }

    let payload = if let Some(item) = item_opt {
        if let Some(synced) = item.synced_lyrics {
            if !synced.trim().is_empty() {
                let lines = parse_lrc(&synced);
                Some(LyricsPayload {
                    synced: true,
                    lines,
                    plain: item.plain_lyrics,
                    source: "LRCLIB".to_string(),
                })
            } else if let Some(plain) = item.plain_lyrics {
                Some(LyricsPayload {
                    synced: false,
                    lines: Vec::new(),
                    plain: Some(plain),
                    source: "LRCLIB".to_string(),
                })
            } else {
                None
            }
        } else if let Some(plain) = item.plain_lyrics {
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

    // Update in-memory cache
    if let Ok(mut cache) = get_cache().lock() {
        cache.insert(key.clone(), payload.clone());
    }

    // Save to disk if found
    if let Some(ref p) = payload {
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
