//! Rich shelf file previews: PDF page-1 rendering via Windows.Data.Pdf,
//! plus Office/OpenDocument embedded thumbnails and XML text snippets.

use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use base64::Engine;

const MAX_OFFICE_FILE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PDF_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_THUMB_ENTRY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_XML_ENTRY_BYTES: usize = 512 * 1024;

/// Renders page 1 of a PDF file to a base64 PNG data URL using `Windows.Data.Pdf`.
pub fn pdf_thumbnail(path: &str) -> Option<String> {
    let p = Path::new(path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if ext != "pdf" {
        return None;
    }
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_PDF_FILE_BYTES {
        return None;
    }

    use windows::core::HSTRING;
    use windows::Data::Pdf::PdfDocument;
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

    let _com = crate::utils::ComGuard::mta();
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).ok()?.get().ok()?;
    let doc = PdfDocument::LoadFromFileAsync(&file).ok()?.get().ok()?;
    if doc.PageCount().ok()? == 0 {
        return None;
    }
    let page = doc.GetPage(0).ok()?;
    let stream = InMemoryRandomAccessStream::new().ok()?;
    page.RenderToStreamAsync(&stream).ok()?.get().ok()?;

    let size = stream.Size().ok()? as u32;
    if size == 0 || size > MAX_THUMB_ENTRY_BYTES as u32 {
        return None;
    }
    let input = stream.GetInputStreamAt(0).ok()?;
    let reader = DataReader::CreateDataReader(&input).ok()?;
    reader.LoadAsync(size).ok()?.get().ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;

    let img = image::load_from_memory(&bytes).ok()?.thumbnail(256, 256);
    let mut png_bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png_bytes)
    ))
}

fn is_office_ext(ext: &str) -> bool {
    matches!(ext, "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp")
}

/// Extracts an embedded thumbnail from an Office Open XML or OpenDocument archive.
pub fn office_thumbnail(path: &str) -> Option<String> {
    let p = Path::new(path);
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !is_office_ext(&ext) {
        return None;
    }
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > MAX_OFFICE_FILE_BYTES {
        return None;
    }

    let file = File::open(p).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let candidates = [
        "docProps/thumbnail.png",
        "docProps/thumbnail.jpeg",
        "docProps/thumbnail.jpg",
        "Thumbnails/thumbnail.png",
    ];
    let target_name = (0..archive.len()).find_map(|i| {
        let entry = archive.by_index(i).ok()?;
        let name = entry.name();
        candidates
            .iter()
            .any(|c| c.eq_ignore_ascii_case(name))
            .then(|| name.to_string())
    })?;

    let entry = archive.by_name(&target_name).ok()?;
    if entry.size() == 0 || entry.size() > MAX_THUMB_ENTRY_BYTES {
        return None;
    }
    let mut buf = Vec::with_capacity(entry.size() as usize);
    entry.take(MAX_THUMB_ENTRY_BYTES).read_to_end(&mut buf).ok()?;

    let img = image::load_from_memory(&buf).ok()?.thumbnail(256, 256);
    let mut png_bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png_bytes)
    ))
}

/// Extracts a short text snippet from Office Open XML or OpenDocument XML content.
pub fn office_text_preview(p: &Path) -> Option<String> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !is_office_ext(&ext) {
        return None;
    }
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > MAX_OFFICE_FILE_BYTES {
        return None;
    }

    let file = File::open(p).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let entry_names: Vec<String> = match ext.as_str() {
        "docx" => vec!["word/document.xml".to_string()],
        "xlsx" => vec!["xl/sharedStrings.xml".to_string()],
        "pptx" => (1..=3).map(|i| format!("ppt/slides/slide{i}.xml")).collect(),
        "odt" | "ods" | "odp" => vec!["content.xml".to_string()],
        _ => return None,
    };

    let mut lines: Vec<String> = Vec::new();
    let mut total_chars = 0usize;

    for name in entry_names {
        let Ok(entry) = archive.by_name(&name) else {
            continue;
        };
        let mut buf = Vec::new();
        if entry.take(MAX_XML_ENTRY_BYTES as u64).read_to_end(&mut buf).is_err() {
            continue;
        }
        let xml = String::from_utf8_lossy(&buf);
        let Ok(doc) = roxmltree::Document::parse(&xml) else {
            continue;
        };
        for node in doc.descendants() {
            if !node.is_text() {
                continue;
            }
            let text = node.text().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }
            lines.push(text.to_string());
            total_chars += text.len();
            if lines.len() >= 8 || total_chars >= 320 {
                break;
            }
        }
        if lines.len() >= 8 || total_chars >= 320 {
            break;
        }
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn extracts_docx_text_and_embedded_thumbnail() {
        let dir = std::env::temp_dir().join(format!("fern-shelf-preview-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let docx_path = dir.join("sample.docx");

        {
            let file = File::create(&docx_path).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();

            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(
                br#"<?xml version="1.0" encoding="UTF-8"?>
                <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
                  <w:body>
                    <w:p><w:r><w:t>Quarterly Report</w:t></w:r></w:p>
                    <w:p><w:r><w:t>Revenue grew 24%.</w:t></w:r></w:p>
                  </w:body>
                </w:document>"#,
            )
            .unwrap();

            // 1x1 PNG
            let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
            let mut png = Vec::new();
            image::DynamicImage::ImageRgba8(img)
                .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                .unwrap();

            zip.start_file("docProps/thumbnail.png", opts).unwrap();
            zip.write_all(&png).unwrap();
            zip.finish().unwrap();
        }

        let text = office_text_preview(&docx_path).expect("docx text preview");
        assert!(text.contains("Quarterly Report"));
        assert!(text.contains("Revenue grew 24%."));

        let thumb = office_thumbnail(docx_path.to_str().unwrap()).expect("docx thumbnail");
        assert!(thumb.starts_with("data:image/png;base64,"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ignores_non_office_extensions() {
        assert!(office_text_preview(Path::new("file.bin")).is_none());
        assert!(office_thumbnail("file.bin").is_none());
        assert!(pdf_thumbnail("file.txt").is_none());
    }
}
