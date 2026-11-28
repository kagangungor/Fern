use base64::{engine::general_purpose, Engine as _};
use windows::Win32::Foundation::{HGLOBAL, HWND};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppPBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::StructuredStorage::{CreateStreamOnHGlobal, GetHGlobalFromStream};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::WindowsAndMessaging::HICON;

use super::settings_io::setting_flag_default_true;
use super::window::get_now_ms;

pub(crate) unsafe fn image_factory_to_base64(
    factory: &windows::Win32::UI::Shell::IShellItemImageFactory,
) -> Option<String> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::UI::Shell::{SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};

    let size = SIZE { cx: 256, cy: 256 };
    let hbitmap = factory
        .GetImage(size, SIIGBF(SIIGBF_ICONONLY.0 | SIIGBF_BIGGERSIZEOK.0))
        .or_else(|_| factory.GetImage(size, SIIGBF_BIGGERSIZEOK))
        .ok()?;
    if hbitmap.0.is_null() {
        return None;
    }
    let b64 = hbitmap_to_base64(hbitmap);
    let _ = DeleteObject(hbitmap.into());
    b64
}

pub unsafe fn icon_from_absolute_pidl(pidl: *const windows::Win32::UI::Shell::Common::ITEMIDLIST) -> Option<String> {
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, SHCreateItemFromIDList, SHFILEINFOW, SHGetFileInfoW,
        SHGFI_ICON, SHGFI_LARGEICON, SHGFI_PIDL,
    };
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    if let Ok(factory) = SHCreateItemFromIDList::<IShellItemImageFactory>(pidl) {
        if let Some(b64) = image_factory_to_base64(&factory) {
            return Some(b64);
        }
    }

    let mut shfi: SHFILEINFOW = std::mem::zeroed();
    let res = SHGetFileInfoW(
        windows::core::PCWSTR(pidl as *const u16),
        Default::default(),
        Some(&mut shfi),
        std::mem::size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON | SHGFI_PIDL,
    );
    if res != 0 && !shfi.hIcon.is_invalid() {
        let b64 = icon_to_base64(shfi.hIcon);
        let _ = DestroyIcon(shfi.hIcon);
        b64
    } else {
        None
    }
}

pub unsafe fn icon_from_parsing_name(name: &str) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::SHParseDisplayName;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut pidl = std::ptr::null_mut();
    SHParseDisplayName(windows::core::PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None).ok()?;
    if pidl.is_null() { return None; }
    let icon = icon_from_absolute_pidl(pidl);
    CoTaskMemFree(Some(pidl as *const _));
    icon
}

pub unsafe fn icon_to_base64(hicon: HICON) -> Option<String> {
    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let bitmap = factory.CreateBitmapFromHICON(hicon).ok()?;

    let stream = CreateStreamOnHGlobal(HGLOBAL(std::ptr::null_mut()), true).ok()?;
    let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;

    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).ok()?;
    let frame = frame?;
    frame.Initialize(None).ok()?;

    let (mut width, mut height) = (0u32, 0u32);
    bitmap.GetSize(&mut width, &mut height).ok()?;
    frame.SetSize(width, height).ok()?;

    let mut format = GUID_WICPixelFormat32bppPBGRA;
    frame.SetPixelFormat(&mut format).ok()?;

    frame.WriteSource(&bitmap, std::ptr::null()).ok()?;
    frame.Commit().ok()?;
    encoder.Commit().ok()?;

    let hglobal = GetHGlobalFromStream(&stream).ok()?;
    let ptr = windows::Win32::System::Memory::GlobalLock(hglobal);
    let size = windows::Win32::System::Memory::GlobalSize(hglobal);

    let data = std::slice::from_raw_parts(ptr as *const u8, size);
    let png_bytes = data.to_vec();

    let _ = windows::Win32::System::Memory::GlobalUnlock(hglobal);

    let normalized = normalize_icon_padding(&png_bytes).unwrap_or(png_bytes);
    let base64_str = general_purpose::STANDARD.encode(&normalized);

    Some(format!("data:image/png;base64,{}", base64_str))
}

// Drops a near-black plate behind an icon (e.g. Figma, Zen).
pub(crate) fn strip_dark_plate(img: &mut image::RgbaImage) {
    let (w, h) = img.dimensions();
    if w < 8 || h < 8 { return; }
    let mut minx = w; let mut miny = h; let mut maxx = 0; let mut maxy = 0;
    for y in 0..h { for x in 0..w {
        if img.get_pixel(x, y)[3] > 200 {
            minx = minx.min(x); maxx = maxx.max(x); miny = miny.min(y); maxy = maxy.max(y);
        }
    } }
    if maxx <= minx + 4 || maxy <= miny + 4 { return; }
    let mut ring: Vec<(u32, u32)> = Vec::new();
    for x in minx..=maxx { ring.push((x, miny + 1)); ring.push((x, maxy - 1)); }
    for y in miny..=maxy { ring.push((minx + 1, y)); ring.push((maxx - 1, y)); }
    let mut counts: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
    for &(x, y) in &ring {
        let p = img.get_pixel(x, y);
        if p[3] > 200 { *counts.entry([p[0], p[1], p[2]]).or_insert(0) += 1; }
    }
    let Some((&bg, &n)) = counts.iter().max_by_key(|(_, c)| **c) else { return };
    let luma = (bg[0] as u32 * 299 + bg[1] as u32 * 587 + bg[2] as u32 * 114) / 1000;
    if luma > 55 || n * 100 < ring.len() * 70 { return; }
    let near = |p: &image::Rgba<u8>| {
        (0..3).all(|i| (p[i] as i32 - bg[i] as i32).abs() <= 8)
    };
    let mut stack: Vec<(u32, u32)> = ring;
    let mut seen = vec![false; (w * h) as usize];
    while let Some((x, y)) = stack.pop() {
        let idx = (y * w + x) as usize;
        if seen[idx] { continue; }
        seen[idx] = true;
        let p = *img.get_pixel(x, y);
        if p[3] == 0 || !near(&p) { continue; }
        img.put_pixel(x, y, image::Rgba([0, 0, 0, 0]));
        if x > 0 { stack.push((x - 1, y)); }
        if y > 0 { stack.push((x, y - 1)); }
        if x + 1 < w { stack.push((x + 1, y)); }
        if y + 1 < h { stack.push((x, y + 1)); }
    }
}

fn normalize_icon_padding(png_bytes: &[u8]) -> Option<Vec<u8>> {
    const TARGET_FILL_RATIO: f64 = 0.82;

    let img = image::load_from_memory(png_bytes).ok()?;
    let mut rgba = img.to_rgba8();
    for px in rgba.pixels_mut() {
        if px[3] < 40 { *px = image::Rgba([0, 0, 0, 0]); }
    }
    let (w, h) = rgba.dimensions();
    if setting_flag_default_true("fern-dock-strip-dark-plate") {
        strip_dark_plate(&mut rgba);
    }

    let mut minx = w;
    let mut miny = h;
    let mut maxx: i64 = 0;
    let mut maxy: i64 = 0;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            if rgba.get_pixel(x, y)[3] > 10 {
                found = true;
                if x < minx { minx = x; }
                if (x as i64) > maxx { maxx = x as i64; }
                if y < miny { miny = y; }
                if (y as i64) > maxy { maxy = y as i64; }
            }
        }
    }
    if !found { return None; }

    let content_w = (maxx as u32).saturating_sub(minx) + 1;
    let content_h = (maxy as u32).saturating_sub(miny) + 1;
    let content_max = content_w.max(content_h);
    let base_size = w.min(h);
    let current_ratio = content_max as f64 / base_size as f64;

    let canvas_size = if current_ratio > TARGET_FILL_RATIO {
        (content_max as f64 / TARGET_FILL_RATIO).round() as u32
    } else {
        base_size
    };

    let cropped = image::imageops::crop_imm(&rgba, minx, miny, content_w, content_h).to_image();
    let mut canvas = image::RgbaImage::new(canvas_size, canvas_size);
    let ox = ((canvas_size - content_w) / 2) as i64;
    let oy = ((canvas_size - content_h) / 2) as i64;
    image::imageops::overlay(&mut canvas, &cropped, ox, oy);

    let mut out: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(canvas)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .ok()?;
    Some(out)
}

fn mostly_painted(pixels: &[u8]) -> bool {
    pixels.chunks_exact(4).step_by(7).filter(|c| c[0] != 0 || c[1] != 0 || c[2] != 0).count() * 50 > pixels.len() / 28
}

pub fn encode_thumbnail(mut pixels: Vec<u8>, width: u32, height: u32, max_width: u32, max_height: u32) -> Option<String> {
    for chunk in pixels.chunks_exact_mut(4) {
        chunk.swap(0, 2);
        chunk[3] = 255;
    }
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut img = image::RgbaImage::from_raw(width, height, pixels)?;
        if img.width() > max_width || img.height() > max_height {
            let dyn_img = image::DynamicImage::ImageRgba8(img);
            img = dyn_img.resize(max_width, max_height, image::imageops::FilterType::Triangle).into_rgba8();
        }
        let mut buf = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(&mut buf, &img, img.width(), img.height(), image::ColorType::Rgba8, image::ImageFormat::Png).ok()?;
        Some(format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(buf.into_inner())))
    }))
    .ok()
    .flatten()
}

/// Captures an HWND to a base64-encoded PNG thumbnail, scaling it down if it exceeds max_width x max_height.
/// The GDI/screen copy goes first; a Windows.Graphics.Capture session per frame leaks GPU memory,
/// so it only runs as a strictly rate-limited fallback when GDI capture fails.
pub fn capture_hwnd_to_base64(hwnd: HWND, max_width: u32, max_height: u32) -> Option<String> {
    capture_hwnd_gdi(hwnd, max_width, max_height).or_else(|| {
        if !wgc_allowed(hwnd) {
            return None;
        }
        crate::diagnostics::log("capture: window copy failed, using Graphics Capture");
        let (pixels, w, h) = crate::capture::capture_window_bgra(hwnd)?;
        if !mostly_painted(&pixels) {
            return None;
        }
        encode_thumbnail(pixels, w, h, max_width, max_height)
    })
}

fn wgc_allowed_with_state(
    total: &std::sync::atomic::AtomicU32,
    map: &mut std::collections::HashMap<isize, i64>,
    hwnd_key: isize,
    now: i64,
) -> bool {
    use std::sync::atomic::Ordering;
    if total.load(Ordering::Relaxed) >= 12
        || map.get(&hwnd_key).is_some_and(|t| now - t < 600_000)
    {
        return false;
    }
    map.insert(hwnd_key, now);
    total.fetch_add(1, Ordering::Relaxed);
    true
}

/// Each Graphics Capture session leaks about 4 MB of GPU memory, so it is limited per window and in total.
fn wgc_allowed(hwnd: HWND) -> bool {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicU32;
    use std::sync::Mutex;
    static TOTAL: AtomicU32 = AtomicU32::new(0);
    static LAST: Mutex<Option<HashMap<isize, i64>>> = Mutex::new(None);
    let now = get_now_ms();
    let Ok(mut guard) = LAST.lock() else { return false };
    let map = guard.get_or_insert_with(Default::default);
    wgc_allowed_with_state(&TOTAL, map, hwnd.0 as isize, now)
}

pub(crate) fn is_chrome_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 64];
    let len = unsafe { windows::Win32::UI::WindowsAndMessaging::GetClassNameW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..len]).starts_with("Chrome_WidgetWin")
}

/// Returns true if any other visible, uncloaked top-level app window sits above `hwnd` in the
/// z-order and overlaps `target_rect`. Used to prevent `BitBlt(hdc_screen)` from capturing
/// another program on top of the target window.
unsafe fn is_window_occluded(hwnd: HWND, target_rect: windows::Win32::Foundation::RECT) -> bool {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, GW_HWNDPREV, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
    };

    let own_pid = std::process::id();
    let mut cur = GetWindow(hwnd, GW_HWNDPREV).unwrap_or_default();
    let mut steps = 0;
    while !cur.is_invalid() && steps < 64 {
        steps += 1;
        if IsWindowVisible(cur).as_bool() && !IsIconic(cur).as_bool() {
            let mut pid = 0u32;
            GetWindowThreadProcessId(cur, Some(&mut pid));
            let ex = GetWindowLongW(cur, GWL_EXSTYLE) as u32;
            let is_tool = (ex & WS_EX_TOOLWINDOW.0) != 0;
            if pid != own_pid && !is_tool {
                let mut cloaked = 0u32;
                let is_cloaked = DwmGetWindowAttribute(
                    cur,
                    DWMWA_CLOAKED,
                    &mut cloaked as *mut _ as *mut _,
                    std::mem::size_of::<u32>() as u32,
                )
                .is_ok()
                    && cloaked != 0;
                if !is_cloaked {
                    let mut r = RECT::default();
                    let has_rect = DwmGetWindowAttribute(
                        cur,
                        DWMWA_EXTENDED_FRAME_BOUNDS,
                        &mut r as *mut _ as *mut _,
                        std::mem::size_of::<RECT>() as u32,
                    )
                    .is_ok()
                        || GetWindowRect(cur, &mut r).is_ok();
                    if has_rect && r.right > r.left && r.bottom > r.top {
                        let ox = r.right.min(target_rect.right) - r.left.max(target_rect.left);
                        let oy = r.bottom.min(target_rect.bottom) - r.top.max(target_rect.top);
                        if ox > 8 && oy > 8 {
                            return true;
                        }
                    }
                }
            }
        }
        cur = GetWindow(cur, GW_HWNDPREV).unwrap_or_default();
    }
    false
}

fn capture_hwnd_gdi(hwnd: HWND, max_width: u32, max_height: u32) -> Option<String> {
    let (pixels, w, h) = capture_hwnd_raw_bgra_inner(hwnd, false)?;
    encode_thumbnail(pixels, w, h, max_width, max_height)
}

/// Fast raw BGRA copy for a window known to be frontmost (e.g. immediately before minimizing).
/// Does NOT resize or PNG-encode on the caller's thread so minimizing stays instantaneous.
pub fn capture_hwnd_raw_bgra_in_front(hwnd: HWND) -> Option<(Vec<u8>, u32, u32)> {
    capture_hwnd_raw_bgra_inner(hwnd, true)
}

fn capture_hwnd_raw_bgra_inner(hwnd: HWND, assume_in_front: bool) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::Graphics::Gdi::{
            CreateCompatibleDC, CreateCompatibleBitmap, SelectObject, DeleteObject, DeleteDC,
            GetDC, ReleaseDC, GetDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HDC,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT, IsWindow};
        use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
        #[link(name = "user32")]
        extern "system" { pub fn PrintWindow(hwnd: HWND, hdcBlt: HDC, nFlags: u32) -> i32; }

        if !IsWindow(Some(hwnd)).as_bool() || windows::Win32::UI::WindowsAndMessaging::IsIconic(hwnd).as_bool() {
            return None;
        }

        let mut rect = RECT::default();
        let _ = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32);
        if rect.right == 0 && rect.bottom == 0 && windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect).is_err() { return None; }
        let mut width = rect.right - rect.left;
        let mut height = rect.bottom - rect.top;
        if width <= 10 || height <= 10 {
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if GetWindowPlacement(hwnd, &mut wp).is_ok() {
                width = wp.rcNormalPosition.right - wp.rcNormalPosition.left;
                height = wp.rcNormalPosition.bottom - wp.rcNormalPosition.top;
            }
        }
        if width <= 100 || height <= 100 || width > 7680 || height > 4320 { return None; }

        let hdc_screen = GetDC(None);
        let hdc_mem = CreateCompatibleDC(Some(hdc_screen));
        let hbm_mem = CreateCompatibleBitmap(hdc_screen, width, height);
        let h_old = SelectObject(hdc_mem, hbm_mem.into());
        // PrintWindow can stall/break Chrome's GPU compositor and leave the window blank, so copy from the screen instead.
        let printed = !is_chrome_window(hwnd) && PrintWindow(hwnd, hdc_mem, 2) != 0;

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [windows::Win32::Graphics::Gdi::RGBQUAD::default(); 1],
        };
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut captured = printed
            && GetDIBits(hdc_mem, hbm_mem, 0, height as u32, Some(pixels.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS) != 0;
        let in_front = (assume_in_front || windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() == hwnd)
            && !is_window_occluded(hwnd, rect);
        match capture_source(captured, captured && is_blank_bgrx(&pixels), in_front) {
            CaptureSource::Printed => {}
            CaptureSource::Screen => {
                use windows::Win32::Graphics::Gdi::{BitBlt, SRCCOPY};
                captured = BitBlt(hdc_mem, 0, 0, width, height, Some(hdc_screen), rect.left, rect.top, SRCCOPY).is_ok()
                    && GetDIBits(hdc_mem, hbm_mem, 0, height as u32, Some(pixels.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS) != 0;
            }
            CaptureSource::Nothing => captured = false,
        }

        SelectObject(hdc_mem, h_old);
        let _ = DeleteObject(hbm_mem.into());
        let _ = DeleteDC(hdc_mem);
        ReleaseDC(None, hdc_screen);

        if captured {
            Some((pixels, width as u32, height as u32))
        } else {
            None
        }
    }
}

#[derive(Debug, PartialEq)]
enum CaptureSource {
    Printed,
    Screen,
    Nothing,
}

/// PrintWindow can't reach an elevated window (Task Manager): it fails or leaves the bitmap black. If
/// that window is in front, what it shows can be copied from the screen instead. Not otherwise, or the
/// thumbnail would show whatever covers it. Better no image than a black one, callers keep their last
/// good thumbnail then.
fn capture_source(printed: bool, blank: bool, in_front: bool) -> CaptureSource {
    if printed && !blank {
        CaptureSource::Printed
    } else if in_front {
        CaptureSource::Screen
    } else {
        CaptureSource::Nothing
    }
}

/// Colour channels only, the alpha byte of a GDI capture means nothing.
fn is_blank_bgrx(pixels: &[u8]) -> bool {
    pixels.chunks_exact(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0)
}

/// Reads an image file and returns it as a base64 PNG data URI.
pub fn image_file_to_base64(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 8 * 1024 * 1024 { return None; }

    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some(format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&bytes)));
    }

    let img = image::load_from_memory(&bytes).ok()?;
    let mut png: Vec<u8> = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(format!("data:image/png;base64,{}", general_purpose::STANDARD.encode(&png)))
}

/// HBITMAP to base64 PNG.
pub unsafe fn hbitmap_to_base64(hbitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<String> {
    let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
    let bitmap = factory.CreateBitmapFromHBITMAP(hbitmap, windows::Win32::Graphics::Gdi::HPALETTE::default(), windows::Win32::Graphics::Imaging::WICBitmapUsePremultipliedAlpha).ok()?;
    wic_bitmap_to_base64(&factory, &bitmap)
}

unsafe fn wic_bitmap_to_base64(factory: &IWICImagingFactory, bitmap: &windows::Win32::Graphics::Imaging::IWICBitmapSource) -> Option<String> {
    let stream = CreateStreamOnHGlobal(HGLOBAL(std::ptr::null_mut()), true).ok()?;
    let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
    encoder.Initialize(&stream, WICBitmapEncoderNoCache).ok()?;

    let mut frame = None;
    encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).ok()?;
    let frame = frame?;
    frame.Initialize(None).ok()?;

    let (mut width, mut height) = (0u32, 0u32);
    bitmap.GetSize(&mut width, &mut height).ok()?;
    frame.SetSize(width, height).ok()?;

    let mut format = GUID_WICPixelFormat32bppPBGRA;
    frame.SetPixelFormat(&mut format).ok()?;

    frame.WriteSource(bitmap, std::ptr::null()).ok()?;
    frame.Commit().ok()?;
    encoder.Commit().ok()?;

    let hglobal = GetHGlobalFromStream(&stream).ok()?;
    let ptr = windows::Win32::System::Memory::GlobalLock(hglobal);
    let size = windows::Win32::System::Memory::GlobalSize(hglobal);

    let data = std::slice::from_raw_parts(ptr as *const u8, size);
    let base64_str = general_purpose::STANDARD.encode(data);

    let _ = windows::Win32::System::Memory::GlobalUnlock(hglobal);

    Some(format!("data:image/png;base64,{}", base64_str))
}

#[cfg(test)]
mod capture_tests {
    use super::{capture_source, is_blank_bgrx, CaptureSource};

    #[test]
    fn a_usable_print_is_kept() {
        assert_eq!(capture_source(true, false, false), CaptureSource::Printed);
        assert_eq!(capture_source(true, false, true), CaptureSource::Printed);
    }

    #[test]
    fn a_failed_or_black_print_uses_the_screen_only_for_the_window_in_front() {
        assert_eq!(capture_source(false, false, true), CaptureSource::Screen);
        assert_eq!(capture_source(true, true, true), CaptureSource::Screen);
    }

    #[test]
    fn a_failed_or_black_print_behind_other_windows_gives_nothing() {
        assert_eq!(capture_source(false, false, false), CaptureSource::Nothing);
        assert_eq!(capture_source(true, true, false), CaptureSource::Nothing);
    }

    #[test]
    fn blank_ignores_alpha() {
        assert!(is_blank_bgrx(&[0, 0, 0, 255, 0, 0, 0, 0]));
        assert!(!is_blank_bgrx(&[0, 0, 0, 255, 0, 1, 0, 0]));
    }

    #[test]
    fn wgc_allowed_enforces_per_window_cooldown_and_total_cap() {
        use std::collections::HashMap;
        use std::sync::atomic::AtomicU32;

        let total = AtomicU32::new(0);
        let mut map = HashMap::new();

        assert!(super::wgc_allowed_with_state(&total, &mut map, 100, 1_000_000));
        assert!(!super::wgc_allowed_with_state(&total, &mut map, 100, 1_599_999));
        assert!(super::wgc_allowed_with_state(&total, &mut map, 100, 1_600_000));

        // 2 calls already made; allow 10 more distinct windows to reach total cap of 12.
        for i in 1..=10 {
            assert!(super::wgc_allowed_with_state(&total, &mut map, 200 + i, 2_000_000));
        }
        assert!(!super::wgc_allowed_with_state(&total, &mut map, 999, 3_000_000));
    }

    #[test]
    fn strip_dark_plate_removes_dark_uniform_border_and_keeps_center() {
        let mut img = image::RgbaImage::from_pixel(16, 16, image::Rgba([20, 20, 24, 255]));
        for y in 5..11 {
            for x in 5..11 {
                img.put_pixel(x, y, image::Rgba([64, 160, 255, 255]));
            }
        }
        super::strip_dark_plate(&mut img);
        assert_eq!(img.get_pixel(0, 0)[3], 0);
        assert_eq!(img.get_pixel(8, 8), &image::Rgba([64, 160, 255, 255]));
    }

    #[test]
    fn strip_dark_plate_preserves_light_backgrounds() {
        let mut img = image::RgbaImage::from_pixel(16, 16, image::Rgba([232, 236, 242, 255]));
        for y in 5..11 {
            for x in 5..11 {
                img.put_pixel(x, y, image::Rgba([30, 30, 30, 255]));
            }
        }
        super::strip_dark_plate(&mut img);
        assert_eq!(img.get_pixel(0, 0), &image::Rgba([232, 236, 242, 255]));
    }
}
