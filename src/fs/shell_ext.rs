use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

const CREATE_NO_WINDOW: u32 = 0x08000000;
const DRIVE_REMOVABLE: u32 = 2;

#[derive(Clone, Debug)]
pub struct SendToItem {
    pub title: String,
    pub target_dir: Option<PathBuf>,
    pub is_desktop_shortcut: bool,
}

#[derive(Clone, Debug)]
pub struct OpenWithItem {
    pub title: String,
    pub exe_path: Option<PathBuf>,
    pub is_default: bool,
    pub is_choose_another: bool,
}

pub fn create_shortcut(target: &Path, link_path: &Path) -> Result<(), String> {
    let target_str = target.to_string_lossy().replace('\'', "''");
    let link_str = link_path.to_string_lossy().replace('\'', "''");

    let script = format!(
        "& {{ $ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut('{}'); $s.TargetPath = '{}'; $s.Save() }}",
        link_str, target_str
    );

    let status = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(&script)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("Shortcut execution error: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("Failed to save shortcut file.".to_string())
    }
}

pub fn get_send_to_destinations() -> Vec<SendToItem> {
    let mut items = Vec::new();

    // 1. Desktop (create shortcut)
    if let Some(user_prof) = std::env::var_os("USERPROFILE") {
        let desktop = PathBuf::from(&user_prof).join("Desktop");
        if desktop.is_dir() {
            items.push(SendToItem {
                title: "Desktop (create shortcut)".to_string(),
                target_dir: Some(desktop),
                is_desktop_shortcut: true,
            });
        }

        // 2. Documents
        let docs = PathBuf::from(&user_prof).join("Documents");
        if docs.is_dir() {
            items.push(SendToItem {
                title: "Documents".to_string(),
                target_dir: Some(docs),
                is_desktop_shortcut: false,
            });
        }
    }

    // 3. Removable USB Drives
    unsafe {
        let drives = GetLogicalDrives();
        for i in 0..26 {
            if (drives & (1 << i)) != 0 {
                let letter = (b'A' + i as u8) as char;
                let root_str = format!("{}:\\", letter);
                let root_w: Vec<u16> = root_str.encode_utf16().chain(std::iter::once(0)).collect();
                if GetDriveTypeW(root_w.as_ptr()) == DRIVE_REMOVABLE {
                    items.push(SendToItem {
                        title: format!("Removable Disk ({}:)", letter),
                        target_dir: Some(PathBuf::from(root_str)),
                        is_desktop_shortcut: false,
                    });
                }
            }
        }
    }

    // 4. Custom SendTo items from %APPDATA%\Microsoft\Windows\SendTo
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let sendto_dir = PathBuf::from(appdata).join(r"Microsoft\Windows\SendTo");
        if let Ok(entries) = std::fs::read_dir(sendto_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if stem.eq_ignore_ascii_case("Desktop (create shortcut)")
                    || stem.eq_ignore_ascii_case("Documents")
                    || stem.eq_ignore_ascii_case("Compressed (zipped) Folder")
                {
                    continue;
                }
                if p.is_dir() {
                    items.push(SendToItem {
                        title: stem,
                        target_dir: Some(p),
                        is_desktop_shortcut: false,
                    });
                }
            }
        }
    }

    items
}

pub fn get_open_with_handlers(path: &Path) -> Vec<OpenWithItem> {
    let mut apps = Vec::new();

    // 1. System Default
    apps.push(OpenWithItem {
        title: "Default App".to_string(),
        exe_path: None,
        is_default: true,
        is_choose_another: false,
    });

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();

    let is_text = matches!(
        ext.as_str(),
        "txt" | "md" | "rs" | "json" | "toml" | "yaml" | "yml" | "html" | "css" | "js" | "ts" | "py" | "c" | "cpp" | "h" | "xml" | "log" | "ini" | "bat" | "cmd" | "ps1" | "csv" | "rc"
    );
    let is_img = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico" | "svg"
    );

    // Standard popular dev and utility tools if available
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let base = PathBuf::from(local_appdata).join("Programs");
        let zed = base.join(r"Zed\Zed.exe");
        if zed.exists() {
            apps.push(OpenWithItem {
                title: "Zed (Code Editor)".to_string(),
                exe_path: Some(zed),
                is_default: false,
                is_choose_another: false,
            });
        }
        let codium = base.join(r"VSCodium\VSCodium.exe");
        if codium.exists() {
            apps.push(OpenWithItem {
                title: "VSCodium".to_string(),
                exe_path: Some(codium),
                is_default: false,
                is_choose_another: false,
            });
        }
    }

    if is_text || apps.len() <= 2 {
        let notepad = PathBuf::from(r"C:\Windows\System32\notepad.exe");
        if notepad.exists() {
            apps.push(OpenWithItem {
                title: "Notepad".to_string(),
                exe_path: Some(notepad),
                is_default: false,
                is_choose_another: false,
            });
        }
    }

    if is_img {
        let paint = PathBuf::from(r"C:\Windows\System32\mspaint.exe");
        if paint.exists() {
            apps.push(OpenWithItem {
                title: "Paint".to_string(),
                exe_path: Some(paint),
                is_default: false,
                is_choose_another: false,
            });
        }
    }

    // "Choose another app..."
    apps.push(OpenWithItem {
        title: "Choose another app...".to_string(),
        exe_path: None,
        is_default: false,
        is_choose_another: true,
    });

    apps
}
