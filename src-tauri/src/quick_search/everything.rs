use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, FindWindowW,
    GetWindowLongPtrW, PeekMessageW, RegisterClassW, SendMessageTimeoutW, SetWindowLongPtrW,
    TranslateMessage, GWLP_USERDATA, HWND_MESSAGE, MSG, PM_REMOVE, SMTO_ABORTIFHUNG, WM_COPYDATA,
    WNDCLASSW,
};

use super::{FileSearchProvider, FileSearchResult};

const EVERYTHING_IPC_COPYDATA_QUERY2W: usize = 18;
const EVERYTHING_IPC_QUERY2_REQUEST_NAME: u32 = 0x0000_0001;
const EVERYTHING_IPC_QUERY2_REQUEST_PATH: u32 = 0x0000_0002;
const EVERYTHING_IPC_FOLDER_FLAG: u32 = 0x0000_0001;
const REPLY_ID: u32 = 0x4645_524E; // "FERN"

#[repr(C)]
struct EverythingIpcQuery2 {
    reply_hwnd: u32,
    reply_copydata_message: u32,
    search_flags: u32,
    offset: u32,
    max_results: u32,
    request_flags: u32,
    sort_type: u32,
}

#[repr(C)]
struct EverythingIpcList2 {
    totfolders: u32,
    totfiles: u32,
    totitems: u32,
    numfolders: u32,
    numfiles: u32,
    numitems: u32,
    offset: u32,
}

#[repr(C)]
struct EverythingIpcItem2 {
    flags: u32,
    data_offset: u32,
}

pub struct EverythingProvider;

impl EverythingProvider {
    fn find_everything_hwnd() -> Option<HWND> {
        let class_name: Vec<u16> = "EVERYTHING_TASKBAR_NOTIFICATION\0".encode_utf16().collect();
        unsafe {
            FindWindowW(PCWSTR(class_name.as_ptr()), PCWSTR::null())
                .ok()
                .filter(|h| !h.0.is_null())
        }
    }

    pub(crate) fn build_query_buffer(reply_hwnd: u32, query: &str, limit: usize) -> Vec<u8> {
        let header = EverythingIpcQuery2 {
            reply_hwnd,
            reply_copydata_message: REPLY_ID,
            search_flags: 0,
            offset: 0,
            max_results: limit.clamp(1, 20) as u32,
            request_flags: EVERYTHING_IPC_QUERY2_REQUEST_NAME | EVERYTHING_IPC_QUERY2_REQUEST_PATH,
            sort_type: 1, // EVERYTHING_IPC_SORT_NAME_ASCENDING
        };
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                (&header as *const EverythingIpcQuery2) as *const u8,
                std::mem::size_of::<EverythingIpcQuery2>(),
            )
        };
        let wide_query: Vec<u16> = query.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = Vec::with_capacity(header_bytes.len() + wide_query.len() * 2);
        buf.extend_from_slice(header_bytes);
        for u in wide_query {
            buf.extend_from_slice(&u.to_le_bytes());
        }
        buf
    }

    pub(crate) fn parse_reply_buffer(data: &[u8]) -> Vec<FileSearchResult> {
        let list_size = std::mem::size_of::<EverythingIpcList2>();
        let item_size = std::mem::size_of::<EverythingIpcItem2>();
        if data.len() < list_size {
            return Vec::new();
        }
        let list = unsafe { std::ptr::read_unaligned(data.as_ptr() as *const EverythingIpcList2) };
        let count = (list.numitems as usize).min(20);
        let items_end = list_size.saturating_add(count.saturating_mul(item_size));
        if data.len() < items_end {
            return Vec::new();
        }

        let mut results = Vec::with_capacity(count);
        for i in 0..count {
            let item_ptr = unsafe {
                data.as_ptr().add(list_size + i * item_size) as *const EverythingIpcItem2
            };
            let item = unsafe { std::ptr::read_unaligned(item_ptr) };
            let mut offset = item.data_offset as usize;
            let Some((name, next_off)) = read_len_prefixed_utf16(data, offset) else {
                continue;
            };
            offset = next_off;
            let Some((parent_dir, _)) = read_len_prefixed_utf16(data, offset) else {
                continue;
            };

            let is_dir = (item.flags & EVERYTHING_IPC_FOLDER_FLAG) != 0;
            let full_path = if parent_dir.is_empty() {
                name.clone()
            } else {
                Path::new(&parent_dir)
                    .join(&name)
                    .to_string_lossy()
                    .to_string()
            };
            let extension = if is_dir {
                None
            } else {
                Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase())
            };
            results.push(FileSearchResult {
                name,
                path: full_path,
                parent_dir,
                is_dir,
                extension,
            });
        }
        results
    }
}

fn read_len_prefixed_utf16(data: &[u8], offset: usize) -> Option<(String, usize)> {
    if offset + 4 > data.len() {
        return None;
    }
    let char_len = u32::from_le_bytes(data[offset..offset + 4].try_into().ok()?) as usize;
    let str_start = offset + 4;
    let byte_len = char_len.checked_mul(2)?;
    let str_end = str_start.checked_add(byte_len)?;
    if str_end > data.len() {
        return None;
    }
    let u16s: Vec<u16> = data[str_start..str_end]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    // Skip trailing null terminator (2 bytes) if present
    let next = (str_end + 2).min(data.len());
    Some((String::from_utf16_lossy(&u16s), next))
}

unsafe extern "system" fn reply_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_COPYDATA && lparam.0 != 0 {
        let cds = &*(lparam.0 as *const COPYDATASTRUCT);
        if cds.dwData == REPLY_ID as usize && !cds.lpData.is_null() && cds.cbData > 0 {
            let user_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const mpsc::Sender<Vec<u8>>;
            if !user_ptr.is_null() {
                let slice =
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let _ = (*user_ptr).send(slice.to_vec());
            }
            return LRESULT(1);
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

impl FileSearchProvider for EverythingProvider {
    fn id(&self) -> &'static str {
        "everything"
    }

    fn is_available(&self) -> bool {
        Self::find_everything_hwnd().is_some()
    }

    fn search(&self, query: &str, limit: usize) -> Vec<FileSearchResult> {
        let trimmed = query.trim();
        if trimmed.chars().count() < 2 || limit == 0 {
            return Vec::new();
        }
        let Some(target_hwnd) = Self::find_everything_hwnd() else {
            return Vec::new();
        };

        unsafe {
            let hinstance = GetModuleHandleW(None).unwrap_or_default();
            let class_name: Vec<u16> = "FernEverythingReplyWnd\0".encode_utf16().collect();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(reply_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            let Ok(reply_hwnd) = CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                Default::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            ) else {
                return Vec::new();
            };

            let (tx, rx) = mpsc::channel::<Vec<u8>>();
            let tx_box = Box::new(tx);
            let tx_ptr = Box::into_raw(tx_box);
            SetWindowLongPtrW(reply_hwnd, GWLP_USERDATA, tx_ptr as isize);

            let mut query_buf = Self::build_query_buffer(reply_hwnd.0 as u32, trimmed, limit);
            let cds = COPYDATASTRUCT {
                dwData: EVERYTHING_IPC_COPYDATA_QUERY2W,
                cbData: query_buf.len() as u32,
                lpData: query_buf.as_mut_ptr() as *mut _,
            };

            let mut send_result = 0usize;
            let sent = SendMessageTimeoutW(
                target_hwnd,
                WM_COPYDATA,
                WPARAM(reply_hwnd.0 as usize),
                LPARAM(&cds as *const COPYDATASTRUCT as isize),
                SMTO_ABORTIFHUNG,
                150,
                Some(&mut send_result),
            );

            let mut reply_bytes = None;
            if sent.0 != 0 {
                let deadline = std::time::Instant::now() + Duration::from_millis(250);
                let mut msg = MSG::default();
                while std::time::Instant::now() < deadline {
                    while PeekMessageW(&mut msg, Some(reply_hwnd), 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    if let Ok(bytes) = rx.try_recv() {
                        reply_bytes = Some(bytes);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(4));
                }
            }

            SetWindowLongPtrW(reply_hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(reply_hwnd);
            drop(Box::from_raw(tx_ptr));

            reply_bytes
                .map(|bytes| Self::parse_reply_buffer(&bytes))
                .unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_buffer_encodes_header_and_null_terminated_utf16() {
        let buf = EverythingProvider::build_query_buffer(1234, "fern", 5);
        assert_eq!(buf.len(), std::mem::size_of::<EverythingIpcQuery2>() + 5 * 2);
    }

    #[test]
    fn parse_reply_handles_empty_or_truncated_buffer_safely() {
        assert!(EverythingProvider::parse_reply_buffer(&[]).is_empty());
        assert!(EverythingProvider::parse_reply_buffer(&[0u8; 8]).is_empty());
    }

    #[test]
    fn search_rejects_too_short_queries_without_panicking() {
        let provider = EverythingProvider;
        assert!(provider.search("", 5).is_empty());
        assert!(provider.search("a", 5).is_empty());
        assert!(provider.search("ş", 5).is_empty());
    }

    #[test]
    fn parse_reply_reads_unaligned_slice_safely() {
        let list_size = std::mem::size_of::<EverythingIpcList2>();
        let item_size = std::mem::size_of::<EverythingIpcItem2>();
        let data_offset = (list_size + item_size) as u32;

        let list = EverythingIpcList2 {
            totfolders: 0,
            totfiles: 1,
            totitems: 1,
            numfolders: 0,
            numfiles: 1,
            numitems: 1,
            offset: 0,
        };
        let item = EverythingIpcItem2 {
            flags: 0,
            data_offset,
        };

        let mut payload = Vec::new();
        // Leading pad byte so &payload[1..] is 1-byte misaligned for u32 fields
        payload.push(0xAA);
        unsafe {
            payload.extend_from_slice(std::slice::from_raw_parts(
                (&list as *const EverythingIpcList2) as *const u8,
                list_size,
            ));
            payload.extend_from_slice(std::slice::from_raw_parts(
                (&item as *const EverythingIpcItem2) as *const u8,
                item_size,
            ));
        }

        let push_len_prefixed = |buf: &mut Vec<u8>, s: &str| {
            let u16s: Vec<u16> = s.encode_utf16().collect();
            buf.extend_from_slice(&(u16s.len() as u32).to_le_bytes());
            for u in u16s {
                buf.extend_from_slice(&u.to_le_bytes());
            }
            buf.extend_from_slice(&0u16.to_le_bytes());
        };
        push_len_prefixed(&mut payload, "report.PDF");
        push_len_prefixed(&mut payload, r"C:\My Docs");

        let unaligned = &payload[1..];
        assert_ne!((unaligned.as_ptr() as usize) % std::mem::align_of::<u32>(), 0);

        let results = EverythingProvider::parse_reply_buffer(unaligned);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "report.PDF");
        assert_eq!(results[0].parent_dir, r"C:\My Docs");
        assert_eq!(results[0].path, r"C:\My Docs\report.PDF");
        assert!(!results[0].is_dir);
        assert_eq!(results[0].extension.as_deref(), Some("pdf"));
    }
}
