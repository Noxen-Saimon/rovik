use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{EM_SCROLLCARET, EM_SETSEL};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static PREVIEW_OPEN: AtomicBool = AtomicBool::new(false);
static mut PREVIEW_HWND: HWND = ptr::null_mut();

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_preview_open() -> bool {
    PREVIEW_OPEN.load(Ordering::Relaxed)
}

pub fn close_preview() {
    unsafe {
        if !PREVIEW_HWND.is_null() {
            DestroyWindow(PREVIEW_HWND);
            PREVIEW_HWND = ptr::null_mut();
        }
    }
    PREVIEW_OPEN.store(false, Ordering::Relaxed);
}

pub fn show_or_toggle_preview(parent: HWND, path: &Path) {
    if is_preview_open() {
        close_preview();
        return;
    }
    open_preview(parent, path);
}

pub fn update_preview_if_open(path: &Path) {
    if is_preview_open() && unsafe { !PREVIEW_HWND.is_null() } {
        unsafe {
            update_preview_content(PREVIEW_HWND, path);
        }
    }
}

struct PreviewData {
    path: PathBuf,
    title: String,
    edit_hwnd: HWND,
    font: HFONT,
    brush_bg: HBRUSH,
}

unsafe fn update_preview_content(hwnd: HWND, path: &Path) {
    let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewData;
    if data_ptr.is_null() {
        return;
    }
    let data = &mut *data_ptr;
    data.path = path.to_path_buf();
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    data.title = format!("Quick Look — {name}");
    SetWindowTextW(hwnd, wide(&data.title).as_ptr());

    let content = generate_preview_text(path);
    SetWindowTextW(data.edit_hwnd, wide(&content).as_ptr());
    SendMessageW(data.edit_hwnd, EM_SETSEL, 0, 0);
    SendMessageW(data.edit_hwnd, EM_SCROLLCARET, 0, 0);
}

fn generate_preview_text(path: &Path) -> String {
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return format!("Cannot read file metadata: {e}"),
    };

    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let size_bytes = metadata.len();
    let size_str = if metadata.is_dir() {
        "Folder".to_string()
    } else if size_bytes < 1024 {
        format!("{size_bytes} B")
    } else if size_bytes < 1024 * 1024 {
        format!("{:.1} KB", size_bytes as f64 / 1024.0)
    } else {
        format!("{:.2} MB", size_bytes as f64 / (1024.0 * 1024.0))
    };

    if metadata.is_dir() {
        let (total_sz, files, dirs) = crate::app::properties_ui::get_folder_metrics(path);
        let sz_formatted = if total_sz < 1024 {
            format!("{total_sz} B")
        } else if total_sz < 1024 * 1024 {
            format!("{:.1} KB", total_sz as f64 / 1024.0)
        } else if total_sz < 1024 * 1024 * 1024 {
            format!("{:.2} MB", total_sz as f64 / 1_048_576.0)
        } else {
            format!("{:.2} GB", total_sz as f64 / 1_073_741_824.0)
        };
        return format!(
            "Folder: {}\r\nLocation: {}\r\nSize: {}\r\nContains: {} files, {} folders",
            name,
            path.display(),
            sz_formatted,
            files,
            dirs
        );
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let is_text = matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "rs"
            | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "xml"
            | "html"
            | "css"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "py"
            | "sh"
            | "bat"
            | "ps1"
            | "ini"
            | "cfg"
            | "log"
            | "gitconfig"
            | "gitignore"
            | "rc"
    );

    if is_text && size_bytes <= 2 * 1024 * 1024 {
        if let Ok(mut f) = File::open(path) {
            let mut buf = Vec::new();
            let mut chunk = vec![0u8; 64 * 1024];
            if let Ok(n) = f.read(&mut chunk) {
                buf.extend_from_slice(&chunk[..n]);
            }
            if let Ok(s) = String::from_utf8(buf) {
                let lines = s.lines().count();
                return format!(
                    "--- {} ({}, {} lines) ---\r\n\r\n{}",
                    name,
                    size_str,
                    lines,
                    s.replace('\n', "\r\n").replace("\r\r\n", "\r\n")
                );
            }
        }
    }

    let is_img = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "ico" | "gif" | "webp");
    if is_img {
        return format!(
            "--- Image Preview ---\r\nFile: {}\r\nFormat: .{}\r\nSize: {}\r\nLocation: {}\r\n\r\n(Double-click or press Enter to open full image)",
            name, ext, size_str, path.display()
        );
    }

    format!(
        "--- File Info ---\r\nFile: {}\r\nType: .{} File\r\nSize: {}\r\nLocation: {}\r\n",
        name, ext, size_str, path.display()
    )
}

unsafe extern "system" fn preview_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            let vk = wparam as u16;
            if vk == VK_SPACE || vk == VK_ESCAPE {
                close_preview();
                return 0;
            }
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let dc = wparam as HDC;
            SetTextColor(dc, 0x00E0E6ED); // Soft white
            SetBkColor(dc, 0x001E1A18);   // Dark slate background
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewData;
            if !data_ptr.is_null() {
                return (*data_ptr).brush_bg as LRESULT;
            }
        }
        WM_SIZE => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewData;
            if !data_ptr.is_null() {
                let mut rect: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rect);
                MoveWindow(
                    (*data_ptr).edit_hwnd,
                    12,
                    12,
                    rect.right - 24,
                    rect.bottom - 24,
                    1,
                );
            }
        }
        WM_DESTROY => {
            let data_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PreviewData;
            if !data_ptr.is_null() {
                let data = Box::from_raw(data_ptr);
                DeleteObject(data.font);
                DeleteObject(data.brush_bg);
            }
            PREVIEW_OPEN.store(false, Ordering::Relaxed);
            PREVIEW_HWND = ptr::null_mut();
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

pub fn open_preview(parent: HWND, path: &Path) {
    unsafe {
        let instance = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(ptr::null());
        let class_name = wide("RovikQuickLookPreview");

        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(preview_wndproc);
        wc.hInstance = instance;
        wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
        wc.hbrBackground = CreateSolidBrush(0x001E1A18);
        wc.lpszClassName = class_name.as_ptr();

        RegisterClassExW(&wc);

        let mut parent_rect: RECT = std::mem::zeroed();
        if !parent.is_null() {
            GetWindowRect(parent, &mut parent_rect);
        } else {
            parent_rect.right = 1000;
            parent_rect.bottom = 700;
        }

        let p_w = parent_rect.right - parent_rect.left;
        let p_h = parent_rect.bottom - parent_rect.top;
        let w = 640.min(p_w - 40);
        let h = 480.min(p_h - 40);
        let x = parent_rect.left + (p_w - w) / 2;
        let y = parent_rect.top + (p_h - h) / 2;

        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let win_title = format!("Quick Look — {name}");

        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class_name.as_ptr(),
            wide(&win_title).as_ptr(),
            WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_VISIBLE,
            x,
            y,
            w,
            h,
            parent,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );

        if hwnd.is_null() {
            return;
        }

        let font = CreateFontW(
            15,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            FIXED_PITCH as u32 | FF_MODERN as u32,
            wide("Consolas").as_ptr(),
        );

        let edit_class = wide("EDIT");
        let edit_hwnd = CreateWindowExW(
            0,
            edit_class.as_ptr(),
            ptr::null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_HSCROLL
                | (ES_MULTILINE as u32)
                | (ES_READONLY as u32)
                | (ES_AUTOVSCROLL as u32)
                | (ES_AUTOHSCROLL as u32),
            12,
            12,
            w - 24,
            h - 24,
            hwnd,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );

        SendMessageW(edit_hwnd, WM_SETFONT, font as WPARAM, 1);

        let data = Box::new(PreviewData {
            path: path.to_path_buf(),
            title: win_title,
            edit_hwnd,
            font,
            brush_bg: CreateSolidBrush(0x001E1A18),
        });

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(data) as isize);
        PREVIEW_HWND = hwnd;
        PREVIEW_OPEN.store(true, Ordering::Relaxed);

        update_preview_content(hwnd, path);
        ShowWindow(hwnd, SW_SHOW);
        SetFocus(hwnd);
    }
}
