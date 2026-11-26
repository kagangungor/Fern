use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmIsCompositionEnabled, DwmQueryThumbnailSourceSize, DwmRegisterThumbnail,
    DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES,
    DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRoundRectRgn, DeleteObject, SetWindowRgn, HBRUSH, HRGN, RGN_OR,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor, GetMessageW,
    GetWindowRect, IsIconic, IsWindow, PostMessageW, RegisterClassW, SetWindowPos, ShowWindow,
    TranslateMessage, GA_ROOT, HTTRANSPARENT, HWND_MESSAGE, HWND_TOPMOST, MA_NOACTIVATE, MSG,
    SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
    WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCHITTEST, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const HOST_CLASS_NAME: PCWSTR = w!("FernLivePreviewHost");
const WM_APP_PROCESS_COMMANDS: u32 = WM_APP + 1;
/// Ellipse diameter for `CreateRoundRectRgn` matching `.preview-surface` `border-radius: 8px`.
const CORNER_DIAMETER_PX: i32 = 16;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LiveThumbnailTarget {
    pub hwnd: isize,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug)]
struct ThumbnailRecord {
    handle: isize,
    rect: RECT,
    opacity: u8,
}

enum HostCommand {
    SetThumbnails {
        dest_hwnd_val: isize,
        targets: Vec<LiveThumbnailTarget>,
        client_area_only: bool,
    },
    SetOpacity(u8),
    ClearAll,
}

enum HostReply {
    Active(Result<Vec<isize>, String>),
    Unit(Result<(), String>),
}

struct HostRequest {
    cmd: HostCommand,
    reply: mpsc::SyncSender<HostReply>,
}

struct HostState {
    thumbnails: HashMap<(isize, isize), ThumbnailRecord>,
    host_windows: HashMap<isize, isize>,
    opacity: u8,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            thumbnails: HashMap::new(),
            host_windows: HashMap::new(),
            opacity: 255,
        }
    }
}

struct HostDispatcher {
    dispatcher_hwnd: isize,
    tx: Mutex<mpsc::Sender<HostRequest>>,
}

static HOST_DISPATCHER: OnceLock<Result<HostDispatcher, String>> = OnceLock::new();

thread_local! {
    static HOST_RX: RefCell<Option<mpsc::Receiver<HostRequest>>> = const { RefCell::new(None) };
    static HOST_STATE: RefCell<HostState> = RefCell::new(HostState::default());
}

fn get_dispatcher() -> Result<&'static HostDispatcher, String> {
    HOST_DISPATCHER
        .get_or_init(spawn_host_thread)
        .as_ref()
        .map_err(|e| e.clone())
}

fn spawn_host_thread() -> Result<HostDispatcher, String> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<HostRequest>();
    let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<isize, String>>(1);

    std::thread::Builder::new()
        .name("fern-live-preview-host".to_string())
        .spawn(move || unsafe {
            let hinstance = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSW {
                lpfnWndProc: Some(host_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: HOST_CLASS_NAME,
                hbrBackground: HBRUSH::default(),
                ..Default::default()
            };
            let _ = RegisterClassW(&wc);

            HOST_RX.with(|cell| {
                *cell.borrow_mut() = Some(cmd_rx);
            });

            let dispatcher = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                HOST_CLASS_NAME,
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            ) {
                Ok(hwnd) => hwnd,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("Failed to create live preview dispatcher: {e}")));
                    return;
                }
            };

            let _ = ready_tx.send(Ok(dispatcher.0 as isize));

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        })
        .map_err(|e| format!("Failed to spawn live preview host thread: {e}"))?;

    let dispatcher_hwnd = ready_rx
        .recv()
        .map_err(|e| format!("Live preview host thread failed to initialize: {e}"))??;

    Ok(HostDispatcher {
        dispatcher_hwnd,
        tx: Mutex::new(cmd_tx),
    })
}

fn send_command(cmd: HostCommand) -> Result<HostReply, String> {
    let dispatcher = get_dispatcher()?;
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    {
        let tx = dispatcher.tx.lock().map_err(|e| e.to_string())?;
        tx.send(HostRequest { cmd, reply: reply_tx })
            .map_err(|e| format!("Live preview host channel closed: {e}"))?;
    }
    unsafe {
        let _ = PostMessageW(
            Some(HWND(dispatcher.dispatcher_hwnd as *mut _)),
            WM_APP_PROCESS_COMMANDS,
            WPARAM(0),
            LPARAM(0),
        );
    }
    reply_rx
        .recv()
        .map_err(|e| format!("Live preview host did not reply: {e}"))
}

unsafe extern "system" fn host_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_APP_PROCESS_COMMANDS => {
            drain_commands();
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn drain_commands() {
    let pending: Vec<HostRequest> = HOST_RX.with(|cell| {
        let borrow = cell.borrow();
        let Some(rx) = borrow.as_ref() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        while let Ok(req) = rx.try_recv() {
            items.push(req);
        }
        items
    });

    HOST_STATE.with(|state_cell| {
        let mut state = state_cell.borrow_mut();
        for req in pending {
            let reply = match req.cmd {
                HostCommand::SetThumbnails {
                    dest_hwnd_val,
                    targets,
                    client_area_only,
                } => HostReply::Active(handle_set_thumbnails(
                    &mut state,
                    dest_hwnd_val,
                    targets,
                    client_area_only,
                )),
                HostCommand::SetOpacity(opacity) => {
                    HostReply::Unit(handle_set_opacity(&mut state, opacity))
                }
                HostCommand::ClearAll => HostReply::Unit(handle_clear_all(&mut state)),
            };
            let _ = req.reply.send(reply);
        }
    });
}

fn ensure_host_window(state: &mut HostState, dest_hwnd_val: isize) -> Result<HWND, String> {
    if let Some(&existing_val) = state.host_windows.get(&dest_hwnd_val) {
        let existing = HWND(existing_val as *mut _);
        if unsafe { IsWindow(Some(existing)).as_bool() } {
            return Ok(existing);
        }
        state.host_windows.remove(&dest_hwnd_val);
    }

    let dest_hwnd = HWND(dest_hwnd_val as *mut _);
    let host_hwnd = unsafe {
        let hinstance = GetModuleHandleW(None).unwrap_or_default();
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_TOPMOST,
            HOST_CLASS_NAME,
            w!(""),
            WS_POPUP,
            0,
            0,
            0,
            0,
            Some(dest_hwnd),
            None,
            Some(hinstance.into()),
            None,
        )
        .map_err(|e| format!("Failed to create live preview host window: {e}"))?
    };

    state.host_windows.insert(dest_hwnd_val, host_hwnd.0 as isize);
    Ok(host_hwnd)
}

fn apply_host_region(host_hwnd: HWND, rects: &[RECT]) {
    if rects.is_empty() {
        return;
    }
    unsafe {
        let mut combined: Option<HRGN> = None;
        for r in rects {
            let rgn = CreateRoundRectRgn(
                r.left,
                r.top,
                r.right + 1,
                r.bottom + 1,
                CORNER_DIAMETER_PX,
                CORNER_DIAMETER_PX,
            );
            if rgn.is_invalid() {
                continue;
            }
            if let Some(acc) = combined {
                let _ = CombineRgn(Some(acc), Some(acc), Some(rgn), RGN_OR);
                let _ = DeleteObject(rgn.into());
            } else {
                combined = Some(rgn);
            }
        }
        if let Some(region) = combined {
            // SetWindowRgn takes ownership of the HRGN on success (non-zero return).
            if SetWindowRgn(host_hwnd, Some(region), true) == 0 {
                let _ = DeleteObject(region.into());
            }
        }
    }
}

pub fn is_composition_enabled() -> bool {
    unsafe {
        DwmIsCompositionEnabled()
            .map(|b| b.as_bool())
            .unwrap_or(false)
    }
}

fn fit_rect(handle: isize, dest: RECT) -> RECT {
    let size = unsafe { DwmQueryThumbnailSourceSize(handle) }.unwrap_or_default();
    let (dw, dh) = (dest.right - dest.left, dest.bottom - dest.top);
    if size.cx <= 0 || size.cy <= 0 || dw <= 0 || dh <= 0 {
        return dest;
    }
    let scale = (dw as f64 / size.cx as f64).min(dh as f64 / size.cy as f64);
    let w = ((size.cx as f64 * scale).round() as i32).clamp(1, dw);
    let h = ((size.cy as f64 * scale).round() as i32).clamp(1, dh);
    let left = dest.left + (dw - w) / 2;
    let top = dest.top + (dh - h) / 2;
    RECT {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

fn handle_set_thumbnails(
    state: &mut HostState,
    dest_hwnd_val: isize,
    targets: Vec<LiveThumbnailTarget>,
    client_area_only: bool,
) -> Result<Vec<isize>, String> {
    let dest_hwnd = HWND(dest_hwnd_val as *mut _);
    unsafe {
        if !IsWindow(Some(dest_hwnd)).as_bool() {
            return Err("Destination window is invalid".to_string());
        }
    }

    // Prune host windows whose owner dock window no longer exists.
    let dead_dests: Vec<isize> = state
        .host_windows
        .keys()
        .copied()
        .filter(|&d| !unsafe { IsWindow(Some(HWND(d as *mut _))).as_bool() })
        .collect();
    for d in dead_dests {
        if let Some(h) = state.host_windows.remove(&d) {
            let hwnd = HWND(h as *mut _);
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }

    let targets: Vec<LiveThumbnailTarget> = targets
        .into_iter()
        .filter(|t| unsafe { IsWindow(Some(HWND(t.hwnd as *mut _))).as_bool() })
        .collect();

    state.opacity = 255;

    let target_hwnds: HashSet<isize> = targets.iter().map(|t| t.hwnd).collect();
    let to_remove: Vec<(isize, isize)> = state
        .thumbnails
        .keys()
        .copied()
        .filter(|&(d_hwnd, src_hwnd_val)| {
            (d_hwnd == dest_hwnd_val && !target_hwnds.contains(&src_hwnd_val))
                || !unsafe { IsWindow(Some(HWND(d_hwnd as *mut _))).as_bool() }
                || !unsafe { IsWindow(Some(HWND(src_hwnd_val as *mut _))).as_bool() }
        })
        .collect();

    for key in to_remove {
        if let Some(record) = state.thumbnails.remove(&key) {
            unsafe {
                let _ = DwmUnregisterThumbnail(record.handle);
            }
        }
    }

    if targets.is_empty() {
        if let Some(&host_val) = state.host_windows.get(&dest_hwnd_val) {
            let host_hwnd = HWND(host_val as *mut _);
            unsafe {
                if IsWindow(Some(host_hwnd)).as_bool() {
                    let _ = ShowWindow(host_hwnd, SW_HIDE);
                }
            }
        }
        return Ok(Vec::new());
    }

    let host_hwnd = ensure_host_window(state, dest_hwnd_val)?;

    let mut dest_rect = RECT::default();
    unsafe {
        if GetWindowRect(dest_hwnd, &mut dest_rect).is_ok() {
            let dest_w = (dest_rect.right - dest_rect.left).max(1);
            let dest_h = (dest_rect.bottom - dest_rect.top).max(1);
            let _ = SetWindowPos(
                host_hwnd,
                Some(HWND_TOPMOST),
                dest_rect.left,
                dest_rect.top,
                dest_w,
                dest_h,
                SWP_NOACTIVATE,
            );
        }
    }

    let mut successfully_active = Vec::new();
    let mut fitted_rects = Vec::new();

    for target in targets {
        let raw_hwnd = HWND(target.hwnd as *mut _);
        let src_hwnd = unsafe {
            if !IsWindow(Some(raw_hwnd)).as_bool() {
                continue;
            }
            let root = GetAncestor(raw_hwnd, GA_ROOT);
            if !root.0.is_null() && IsWindow(Some(root)).as_bool() {
                root
            } else {
                raw_hwnd
            }
        };

        let rc_dest = RECT {
            left: target.x,
            top: target.y,
            right: target.x + target.width.max(1),
            bottom: target.y + target.height.max(1),
        };

        let key = (dest_hwnd_val, target.hwnd);
        let existing_stale = state.thumbnails.get(&key).map(|existing| unsafe {
            let size = DwmQueryThumbnailSourceSize(existing.handle).unwrap_or_default();
            let is_iconic = IsIconic(src_hwnd).as_bool() || IsIconic(raw_hwnd).as_bool();
            (is_iconic && (size.cx < 64 || size.cy < 64)) || size.cx < 64 || size.cy < 64
        });

        if existing_stale == Some(true) {
            if let Some(record) = state.thumbnails.remove(&key) {
                unsafe {
                    let _ = DwmUnregisterThumbnail(record.handle);
                }
            }
            continue;
        }

        if let Some(existing) = state.thumbnails.get_mut(&key) {
            let fitted = fit_rect(existing.handle, rc_dest);
            if existing.rect != fitted || existing.opacity != 255 {
                existing.rect = fitted;
                existing.opacity = 255;
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION
                        | DWM_TNP_VISIBLE
                        | DWM_TNP_OPACITY
                        | DWM_TNP_SOURCECLIENTAREAONLY,
                    rcDestination: fitted,
                    rcSource: RECT::default(),
                    opacity: 255,
                    fVisible: BOOL(1),
                    fSourceClientAreaOnly: if client_area_only { BOOL(1) } else { BOOL(0) },
                };
                unsafe {
                    let _ = DwmUpdateThumbnailProperties(existing.handle, &props);
                }
            }
            successfully_active.push(target.hwnd);
            fitted_rects.push(fitted);
        } else {
            unsafe {
                if let Ok(handle) = DwmRegisterThumbnail(host_hwnd, src_hwnd) {
                    if IsIconic(raw_hwnd).as_bool() || IsIconic(src_hwnd).as_bool() {
                        let size = DwmQueryThumbnailSourceSize(handle).unwrap_or_default();
                        if size.cx < 64 || size.cy < 64 {
                            let _ = DwmUnregisterThumbnail(handle);
                            continue;
                        }
                    }
                    let fitted = fit_rect(handle, rc_dest);
                    let props = DWM_THUMBNAIL_PROPERTIES {
                        dwFlags: DWM_TNP_RECTDESTINATION
                            | DWM_TNP_VISIBLE
                            | DWM_TNP_OPACITY
                            | DWM_TNP_SOURCECLIENTAREAONLY,
                        rcDestination: fitted,
                        rcSource: RECT::default(),
                        opacity: 255,
                        fVisible: BOOL(1),
                        fSourceClientAreaOnly: if client_area_only { BOOL(1) } else { BOOL(0) },
                    };
                    let _ = DwmUpdateThumbnailProperties(handle, &props);
                    state.thumbnails.insert(
                        key,
                        ThumbnailRecord {
                            handle,
                            rect: fitted,
                            opacity: 255,
                        },
                    );
                    successfully_active.push(target.hwnd);
                    fitted_rects.push(fitted);
                }
            }
        }
    }

    unsafe {
        if !fitted_rects.is_empty() {
            apply_host_region(host_hwnd, &fitted_rects);
            let _ = ShowWindow(host_hwnd, SW_SHOWNOACTIVATE);
        } else {
            let _ = ShowWindow(host_hwnd, SW_HIDE);
        }
    }

    Ok(successfully_active)
}

fn handle_set_opacity(state: &mut HostState, opacity: u8) -> Result<(), String> {
    state.opacity = opacity;
    for record in state.thumbnails.values_mut() {
        record.opacity = opacity;
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_OPACITY,
            opacity,
            ..Default::default()
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(record.handle, &props);
        }
    }

    for (&dest_val, &host_val) in &state.host_windows {
        let host_hwnd = HWND(host_val as *mut _);
        unsafe {
            if !IsWindow(Some(host_hwnd)).as_bool() {
                continue;
            }
            if opacity == 0 {
                let _ = ShowWindow(host_hwnd, SW_HIDE);
            } else {
                let has_active = state.thumbnails.keys().any(|&(d, _)| d == dest_val);
                let _ = ShowWindow(
                    host_hwnd,
                    if has_active {
                        SW_SHOWNOACTIVATE
                    } else {
                        SW_HIDE
                    },
                );
            }
        }
    }

    Ok(())
}

fn handle_clear_all(state: &mut HostState) -> Result<(), String> {
    state.opacity = 255;
    for (_, record) in state.thumbnails.drain() {
        unsafe {
            let _ = DwmUnregisterThumbnail(record.handle);
        }
    }
    for &host_val in state.host_windows.values() {
        let host_hwnd = HWND(host_val as *mut _);
        unsafe {
            if IsWindow(Some(host_hwnd)).as_bool() {
                let _ = ShowWindow(host_hwnd, SW_HIDE);
            }
        }
    }
    Ok(())
}

pub fn set_live_thumbnails(
    dest_hwnd_val: isize,
    targets: Vec<LiveThumbnailTarget>,
    client_area_only: bool,
) -> Result<Vec<isize>, String> {
    if !is_composition_enabled() {
        return Err("DWM composition is not enabled".to_string());
    }

    match send_command(HostCommand::SetThumbnails {
        dest_hwnd_val,
        targets,
        client_area_only,
    })? {
        HostReply::Active(res) => res,
        HostReply::Unit(_) => Err("Unexpected reply from live preview host".to_string()),
    }
}

pub fn set_opacity(opacity: u8) -> Result<(), String> {
    if HOST_DISPATCHER.get().is_none() {
        return Ok(());
    }
    match send_command(HostCommand::SetOpacity(opacity))? {
        HostReply::Unit(res) => res,
        HostReply::Active(_) => Err("Unexpected reply from live preview host".to_string()),
    }
}

pub fn clear_all_thumbnails() -> Result<(), String> {
    if HOST_DISPATCHER.get().is_none() {
        return Ok(());
    }
    match send_command(HostCommand::ClearAll)? {
        HostReply::Unit(res) => res,
        HostReply::Active(_) => Err("Unexpected reply from live preview host".to_string()),
    }
}
