#![allow(dead_code)]

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{resize, FilterType};
use image::{ExtendedColorType, RgbImage};
use tauri::Manager;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetMonitorInfoW, MonitorFromPoint, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, MONITORINFO, MONITOR_DEFAULTTONEAREST, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetSystemMetrics, GetTopWindow, GetWindow, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetWindowDisplayAffinity, GW_HWNDNEXT,
    SM_CXSCREEN, SM_CYSCREEN, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
};

const MAX_LONG_EDGE: u32 = 1280;
const JPEG_QUALITY: u8 = 82;
const MIN_WINDOW_DIM: i32 = 100;

/// RAII guard that excludes the main Notch window from screen/window capture
/// and unconditionally restores `WDA_NONE` on drop.
struct NotchExclusionGuard {
    hwnd: Option<HWND>,
}

impl NotchExclusionGuard {
    fn acquire(app: &tauri::AppHandle) -> Self {
        let hwnd = app
            .get_webview_window("main")
            .and_then(|win| win.hwnd().ok())
            .map(|h| HWND(h.0));

        let active_hwnd = hwnd.filter(|&h| unsafe {
            SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE).is_ok()
        });

        Self { hwnd: active_hwnd }
    }
}

impl Drop for NotchExclusionGuard {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd {
            let _ = unsafe { SetWindowDisplayAffinity(hwnd, WDA_NONE) };
        }
    }
}

struct ScreenDcGuard(HDC);

impl Drop for ScreenDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                ReleaseDC(None, self.0);
            }
        }
    }
}

struct MemDcGuard(HDC);

impl Drop for MemDcGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteDC(self.0);
            }
        }
    }
}

struct BitmapGuard(HBITMAP);

impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.0.0));
            }
        }
    }
}

struct SelectGuard {
    dc: HDC,
    prev: HGDIOBJ,
}

impl Drop for SelectGuard {
    fn drop(&mut self) {
        if !self.dc.is_invalid() && !self.prev.is_invalid() {
            unsafe {
                SelectObject(self.dc, self.prev);
            }
        }
    }
}

/// Captures either the active monitor (`"screen"`) or the top-most non-Fern window (`"window"`),
/// excluding the Fern Notch from the frame, and returns a `data:image/jpeg;base64,...` data URL
/// resized to at most 1280px on the long edge at JPEG quality 82.
pub fn capture_for_ai(app: &tauri::AppHandle, mode: &str) -> Result<String, String> {
    let _notch_guard = NotchExclusionGuard::acquire(app);

    let (bgra, width, height) = match mode {
        "screen" => capture_screen_bgra()?,
        "window" => {
            if let Some(target_hwnd) = find_target_window() {
                if let Some(captured) = crate::capture::capture_window_bgra(target_hwnd) {
                    captured
                } else {
                    capture_screen_bgra()?
                }
            } else {
                capture_screen_bgra()?
            }
        }
        other => return Err(format!("Unsupported capture mode: {other}")),
    };

    encode_bgra_to_jpeg_data_url(&bgra, width, height)
}

/// Converts a top-down BGRA pixel buffer to a base64 JPEG data URL (`data:image/jpeg;base64,...`),
/// scaling proportionally if the longest edge exceeds 1280px.
pub fn encode_bgra_to_jpeg_data_url(
    bgra: &[u8],
    width: u32,
    height: u32,
) -> Result<String, String> {
    if width == 0 || height == 0 {
        return Err("Capture dimensions must be greater than zero".to_string());
    }

    let pixel_count = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| "Capture dimensions overflow".to_string())?;
    let expected_bytes = pixel_count
        .checked_mul(4)
        .ok_or_else(|| "Capture buffer size overflow".to_string())?;

    if bgra.len() < expected_bytes {
        return Err(format!(
            "BGRA buffer too small: got {} bytes, expected at least {}",
            bgra.len(),
            expected_bytes
        ));
    }

    let mut rgb = Vec::with_capacity(pixel_count * 3);
    for chunk in bgra[..expected_bytes].chunks_exact(4) {
        rgb.push(chunk[2]); // R
        rgb.push(chunk[1]); // G
        rgb.push(chunk[0]); // B
    }

    let img = RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| "Failed to build RGB image from BGRA buffer".to_string())?;

    let long_edge = width.max(height);
    let final_img = if long_edge > MAX_LONG_EDGE {
        let scale = MAX_LONG_EDGE as f64 / long_edge as f64;
        let new_w = ((width as f64 * scale).round() as u32).clamp(1, MAX_LONG_EDGE);
        let new_h = ((height as f64 * scale).round() as u32).clamp(1, MAX_LONG_EDGE);
        resize(&img, new_w, new_h, FilterType::Triangle)
    } else {
        img
    };

    let mut jpeg_buf = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut jpeg_buf, JPEG_QUALITY);
    encoder
        .encode(
            final_img.as_raw(),
            final_img.width(),
            final_img.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("JPEG encoding failed: {e}"))?;

    Ok(format!("data:image/jpeg;base64,{}", BASE64.encode(&jpeg_buf)))
}

fn active_monitor_rect() -> Result<(i32, i32, i32, i32), String> {
    unsafe {
        let mut cursor = POINT::default();
        if GetCursorPos(&mut cursor).is_ok() {
            let hmon = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            if !hmon.is_invalid() {
                let mut info = MONITORINFO {
                    cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                    ..Default::default()
                };
                if GetMonitorInfoW(hmon, &mut info).as_bool() {
                    let rc = info.rcMonitor;
                    let w = rc.right - rc.left;
                    let h = rc.bottom - rc.top;
                    if w > 0 && h > 0 {
                        return Ok((rc.left, rc.top, w, h));
                    }
                }
            }
        }

        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        if w > 0 && h > 0 {
            Ok((0, 0, w, h))
        } else {
            Err("Failed to determine screen dimensions".to_string())
        }
    }
}

fn capture_screen_bgra() -> Result<(Vec<u8>, u32, u32), String> {
    let (x, y, width, height) = active_monitor_rect()?;

    unsafe {
        let screen_dc = ScreenDcGuard(GetDC(None));
        if screen_dc.0.is_invalid() {
            return Err("GetDC(None) failed".to_string());
        }

        let mem_dc = MemDcGuard(CreateCompatibleDC(Some(screen_dc.0)));
        if mem_dc.0.is_invalid() {
            return Err("CreateCompatibleDC failed".to_string());
        }

        let bitmap = BitmapGuard(CreateCompatibleBitmap(screen_dc.0, width, height));
        if bitmap.0.is_invalid() {
            return Err("CreateCompatibleBitmap failed".to_string());
        }

        let prev_obj = SelectObject(mem_dc.0, HGDIOBJ(bitmap.0.0));
        if prev_obj.is_invalid() {
            return Err("SelectObject failed".to_string());
        }
        let select_guard = SelectGuard {
            dc: mem_dc.0,
            prev: prev_obj,
        };

        BitBlt(
            mem_dc.0,
            0,
            0,
            width,
            height,
            Some(screen_dc.0),
            x,
            y,
            SRCCOPY,
        )
        .map_err(|e| format!("BitBlt failed: {e}"))?;

        // Win32 GDI requires the bitmap to be deselected from the DC before calling GetDIBits.
        drop(select_guard);

        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buf = vec![0u8; (width as usize) * (height as usize) * 4];
        let scanlines = GetDIBits(
            mem_dc.0,
            bitmap.0,
            0,
            height as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        if scanlines == 0 {
            return Err("GetDIBits failed".to_string());
        }

        Ok((buf, width as u32, height as u32))
    }
}

fn is_capturable_window(hwnd: HWND, own_pid: u32) -> bool {
    if hwnd.is_invalid() {
        return false;
    }

    unsafe {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 || pid == own_pid {
            return false;
        }

        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return false;
        }

        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut _,
            std::mem::size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return false;
        }

        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_err() {
            return false;
        }

        (rect.right - rect.left) >= MIN_WINDOW_DIM && (rect.bottom - rect.top) >= MIN_WINDOW_DIM
    }
}

fn walk_z_order(start: HWND, own_pid: u32) -> Option<HWND> {
    let mut cur = start;
    while !cur.is_invalid() {
        if is_capturable_window(cur, own_pid) {
            return Some(cur);
        }
        cur = match unsafe { GetWindow(cur, GW_HWNDNEXT) } {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    None
}

fn find_target_window() -> Option<HWND> {
    let own_pid = std::process::id();
    let fg = unsafe { GetForegroundWindow() };
    if let Some(found) = walk_z_order(fg, own_pid) {
        return Some(found);
    }

    let top = unsafe { GetTopWindow(None) }.ok()?;
    walk_z_order(top, own_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_bgra_to_jpeg_resizes_to_1280_max_edge() {
        let width: u32 = 2000;
        let height: u32 = 1000;
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        for (i, pixel) in bgra.chunks_exact_mut(4).enumerate() {
            pixel[0] = (i % 255) as u8; // B
            pixel[1] = ((i / 255) % 255) as u8; // G
            pixel[2] = 200; // R
            pixel[3] = 255; // A
        }

        let data_url =
            encode_bgra_to_jpeg_data_url(&bgra, width, height).expect("encoding should succeed");
        let b64 = data_url
            .strip_prefix("data:image/jpeg;base64,")
            .expect("should have JPEG data URL prefix");
        let jpeg_bytes = BASE64.decode(b64).expect("base64 decode should succeed");
        let decoded = image::load_from_memory(&jpeg_bytes).expect("JPEG decode should succeed");

        assert_eq!(decoded.width(), 1280);
        assert_eq!(decoded.height(), 640);
    }

    #[test]
    fn encode_bgra_to_jpeg_preserves_small_dimensions_and_channel_order() {
        let width: u32 = 64;
        let height: u32 = 32;
        // Solid red in BGRA: B=0, G=0, R=255, A=255
        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            bgra.extend_from_slice(&[0, 0, 255, 255]);
        }

        let data_url =
            encode_bgra_to_jpeg_data_url(&bgra, width, height).expect("encoding should succeed");
        let b64 = data_url
            .strip_prefix("data:image/jpeg;base64,")
            .expect("should have JPEG data URL prefix");
        let jpeg_bytes = BASE64.decode(b64).expect("base64 decode should succeed");
        let decoded = image::load_from_memory(&jpeg_bytes)
            .expect("JPEG decode should succeed")
            .to_rgb8();

        assert_eq!(decoded.width(), 64);
        assert_eq!(decoded.height(), 32);
        let px = decoded.get_pixel(32, 16);
        assert!(px[0] > 230, "expected high R channel, got {:?}", px);
        assert!(px[1] < 25, "expected low G channel, got {:?}", px);
        assert!(px[2] < 25, "expected low B channel, got {:?}", px);
    }

    #[test]
    fn encode_bgra_to_jpeg_rejects_invalid_inputs() {
        assert!(encode_bgra_to_jpeg_data_url(&[], 0, 100).is_err());
        assert!(encode_bgra_to_jpeg_data_url(&[], 100, 0).is_err());
        assert!(encode_bgra_to_jpeg_data_url(&[0u8; 10], 10, 10).is_err());
    }
}
