use tauri::AppHandle;

#[tauri::command]
pub fn setup_shelf_drop(app: AppHandle) {
    let first = app.clone();
    let _ = app.run_on_main_thread(move || crate::shelf::setup(&first));
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
        let late = app.clone();
        let _ = app.run_on_main_thread(move || crate::shelf::setup(&late));
    });
}

#[tauri::command]
pub async fn shelf_thumbnail(path: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || {
        shelf_image_thumbnail(&path)
            .or_else(|| crate::shelf_preview::pdf_thumbnail(&path))
            .or_else(|| crate::shelf_preview::office_thumbnail(&path))
            .or_else(|| shelf_shell_thumbnail(&path))
    })
    .await
    .ok()
    .flatten()
}

pub(crate) fn extract_shelf_text_preview(p: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(
        ext.as_str(),
        "txt" | "md" | "markdown" | "log" | "json" | "csv" | "tsv" | "xml" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf"
            | "html" | "htm" | "css" | "js" | "jsx" | "ts" | "tsx" | "rs" | "py" | "go" | "java" | "c" | "h" | "cpp" | "cs"
            | "sh" | "ps1" | "bat" | "sql" | "rtf"
    ) {
        return crate::shelf_preview::office_text_preview(p);
    }
    let mut buf = vec![0u8; 1024];
    let n = std::fs::File::open(p).ok()?.read(&mut buf).ok()?;
    if n == 0 || buf[..n].contains(&0) {
        return None;
    }
    let text = String::from_utf8_lossy(&buf[..n]);
    let preview: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).take(8).collect();
    if preview.is_empty() { None } else { Some(preview.join("\n")) }
}

#[tauri::command]
pub async fn shelf_text_preview(path: String) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || extract_shelf_text_preview(std::path::Path::new(&path)))
        .await
        .ok()
        .flatten()
}

fn shelf_shell_thumbnail(path: &str) -> Option<String> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY};

    let _com = crate::utils::ComGuard::mta();
    unsafe {
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(&windows::core::HSTRING::from(path), None).ok()?;
        let hbitmap = factory
            .GetImage(SIZE { cx: 256, cy: 256 }, SIIGBF(SIIGBF_THUMBNAILONLY.0 | SIIGBF_BIGGERSIZEOK.0))
            .ok()?;
        if hbitmap.0.is_null() {
            return None;
        }
        let result = crate::utils::hbitmap_to_base64(hbitmap);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(hbitmap.into());
        result
    }
}

fn shelf_image_thumbnail(path: &str) -> Option<String> {
    use base64::Engine;

    let p = std::path::Path::new(path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff") {
        return None;
    }
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > 32 * 1024 * 1024 {
        return None;
    }

    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    let mut reader = image::ImageReader::open(p).ok()?.with_guessed_format().ok()?;
    reader.limits(limits);
    let thumb = reader.decode().ok()?.thumbnail(256, 256);
    let mut png_bytes: Vec<u8> = Vec::new();
    thumb.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png_bytes)))
}

#[tauri::command]
pub async fn shelf_open(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn shelf_prune(paths: Vec<String>) -> Vec<String> {
    paths.into_iter().filter(|p| std::path::Path::new(p).exists()).collect()
}

#[tauri::command]
pub fn shelf_drag_icon() -> Option<String> {
    use base64::Engine;

    let page = image::Rgba([246, 246, 246, 255]);
    let fold = image::Rgba([198, 198, 198, 255]);
    let line = image::Rgba([152, 152, 152, 255]);
    let mut img = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 0, 0]));
    for y in 6..58u32 {
        for x in 14..50u32 {
            let in_fold = x >= 38 && y <= 18 && (x - 38) + (y - 6) >= 12;
            img.put_pixel(x, y, if in_fold { fold } else { page });
        }
    }
    for (i, y) in [22u32, 30, 38, 46].into_iter().enumerate() {
        let end = if i == 3 { 36 } else { 42 };
        for x in 20..end {
            img.put_pixel(x, y, line);
        }
    }
    let mut png_bytes: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png_bytes)))
}

#[tauri::command]
pub fn shelf_text_file(text: String) -> Option<String> {
    let name: String = text
        .trim()
        .chars()
        .take(24)
        .map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let name = if name.trim().is_empty() { "note".to_string() } else { name.trim().to_string() };
    let dir = std::env::temp_dir().join("fern-shelf");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(format!("{name}.txt"));
    std::fs::write(&path, text).ok()?;
    Some(path.to_string_lossy().into_owned())
}
