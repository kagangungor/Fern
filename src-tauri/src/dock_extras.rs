use windows::core::PCWSTR;
use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHEmptyRecycleBinW, SHQUERYRBINFO};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct RecycleBinStatus {
    pub is_empty: bool,
    pub item_count: i64,
    pub size_bytes: i64,
}

#[tauri::command]
pub fn get_recycle_bin_status() -> Result<RecycleBinStatus, String> {
    unsafe {
        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            i64Size: 0,
            i64NumItems: 0,
        };
        let hr = SHQueryRecycleBinW(PCWSTR::null(), &mut info);
        if hr.is_err() {
            return Err(format!("SHQueryRecycleBinW failed: {:?}", hr));
        }
        Ok(RecycleBinStatus {
            is_empty: info.i64NumItems == 0,
            item_count: info.i64NumItems,
            size_bytes: info.i64Size,
        })
    }
}

#[tauri::command]
pub fn empty_recycle_bin(confirm: Option<bool>) -> Result<(), String> {
    unsafe {
        // SHERB_NOCONFIRMATION = 0x00000001
        // SHERB_NOPROGRESSUI = 0x00000002
        // SHERB_NOSOUND = 0x00000004
        // If confirm is false or Some(false), silent empty.
        // If confirm is true or Some(true), show Windows confirmation dialog.
        let flags: u32 = if confirm.unwrap_or(false) {
            0
        } else {
            0x00000001 // SHERB_NOCONFIRMATION
        };
        let hr = SHEmptyRecycleBinW(None, PCWSTR::null(), flags);
        if hr.is_err() {
            // User cancelling dialog returns S_FALSE or HRESULT corresponding to cancelled
            return Err(format!("SHEmptyRecycleBinW failed: {:?}", hr));
        }
        Ok(())
    }
}

#[tauri::command]
pub fn eject_drive(letter: String) -> Result<(), String> {
    let clean_letter = letter.trim().to_uppercase().chars().next().ok_or("Invalid drive letter")?;
    if !clean_letter.is_ascii_alphabetic() {
        return Err("Drive letter must be A-Z".to_string());
    }

    let cmd = format!(
        "$d = (New-Object -comObject Shell.Application).Namespace(17).ParseName('{clean_letter}:'); if ($d) {{ $d.InvokeVerb('Eject') }} else {{ exit 1 }}"
    );

    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &cmd])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to eject drive {clean_letter}: {err}"));
    }

    Ok(())
}

#[tauri::command]
pub fn shelf_read_text_preview(path: String) -> Result<String, String> {
    let p = std::path::Path::new(&path);
    if !p.exists() || !p.is_file() {
        return Err("File not found".to_string());
    }
    use std::io::Read;
    let mut file = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 4096];
    let n = file.read(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(n);
    let s = String::from_utf8_lossy(&buf).to_string();
    Ok(s)
}
