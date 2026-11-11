use std::{
    fs,
    mem::zeroed,
    path::{Path, PathBuf},
    ptr,
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, DeleteObject, EndPaint, GetStockObject, InvalidateRect,
        DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK,
        FW_NORMAL, FW_SEMIBOLD, HFONT, NULL_BRUSH, PAINTSTRUCT,
    },
    Storage::FileSystem::{
        GetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY, FILE_ATTRIBUTE_SYSTEM,
        INVALID_FILE_ATTRIBUTES,
    },
    UI::{
        Controls::DRAWITEMSTRUCT,
        Input::KeyboardAndMouse::VK_ESCAPE,
        Shell::{ShellExecuteExW, SEE_MASK_INVOKEIDLIST, SHELLEXECUTEINFOW},
        WindowsAndMessaging::*,
    },
};

const DLG_W: i32 = 480;
const DLG_H: i32 = 560;

const ID_BTN_COPY_PATH: usize = 2001;
const ID_BTN_COPY_SHA: usize = 2002;
const ID_BTN_SYS_PROPS: usize = 2003;
const ID_BTN_CLOSE: usize = 2004;

pub struct PropertiesState {
    path: PathBuf,
    name: String,
    is_dir: bool,
    is_drive: bool,
    file_type_desc: String,
    location: String,
    size_display: String,
    contains_display: Option<String>,
    created_display: String,
    modified_display: String,
    accessed_display: String,
    attributes_display: String,
    sha256: Option<String>,
    status_feedback: Option<String>,

    font_normal: HFONT,
    font_bold: HFONT,
    font_title: HFONT,
    font_small: HFONT,
    font_mono: HFONT,
}

fn format_system_time(st: Option<SystemTime>) -> String {
    let time = match st {
        Some(t) => t,
        None => return "-".to_string(),
    };

    let dur = match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d,
        Err(_) => return "-".to_string(),
    };

    let utc_secs = dur.as_secs() as i64;
    let local_secs = (utc_secs + crate::app::TIMEZONE_OFFSET_SECS).max(0) as u64;
    // Days since UNIX epoch in local time (UTC+7 Jakarta)
    let days = (local_secs / 86400) as i64;
    let day_secs = local_secs % 86400;
    let hours = day_secs / 3600;
    let mins = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    let z = days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + (if mp < 10 { 3 } else { -9 });
    let final_year = year + (if month <= 2 { 1 } else { 0 });

    format!("{final_year:04}-{month:02}-{day:02} {hours:02}:{mins:02}:{s:02}")
}

pub fn get_folder_metrics(path: &Path) -> (u64, usize, usize) {
    let mut total_size = 0u64;
    let mut files_count = 0usize;
    let mut dirs_count = 0usize;

    fn walk(p: &Path, sz: &mut u64, files: &mut usize, dirs: &mut usize, depth: usize) {
        if depth > 12 {
            return;
        }
        if let Ok(entries) = fs::read_dir(p) {
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        *dirs += 1;
                        walk(&entry.path(), sz, files, dirs, depth + 1);
                    } else if ft.is_file() {
                        *files += 1;
                        if let Ok(meta) = entry.metadata() {
                            *sz += meta.len();
                        }
                    }
                }
            }
        }
    }

    walk(path, &mut total_size, &mut files_count, &mut dirs_count, 0);
    (total_size, files_count, dirs_count)
}

fn open_windows_properties(hwnd: HWND, path: &Path) {
    let prop_w = crate::app::wide("properties");
    let path_w = crate::app::wide(&path.to_string_lossy());
    unsafe {
        let mut info: SHELLEXECUTEINFOW = zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_INVOKEIDLIST;
        info.hwnd = hwnd;
        info.lpVerb = prop_w.as_ptr();
        info.lpFile = path_w.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        ShellExecuteExW(&mut info);
    }
}

pub fn show_properties_dialog(owner: HWND, target_path: &Path) {
    let is_dir = target_path.is_dir();
    let is_drive = target_path.parent().is_none()
        || target_path.to_string_lossy().ends_with(":\\")
        || target_path.to_string_lossy().ends_with(":/");

    let name = if let Some(n) = target_path.file_name() {
        n.to_string_lossy().to_string()
    } else {
        target_path.to_string_lossy().to_string()
    };

    let parent_str = target_path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| target_path.to_string_lossy().to_string());

    let meta = fs::metadata(target_path).ok();

    let (size_display, contains_display) = if is_dir {
        let (total_sz, files, dirs) = get_folder_metrics(target_path);
        let sz_formatted = crate::app::format_size(total_sz, true);
        (
            format!("{sz_formatted} ({total_sz} bytes)"),
            Some(format!("{files} Files, {dirs} Folders")),
        )
    } else if let Some(ref m) = meta {
        let sz = m.len();
        let sz_formatted = crate::app::format_size(sz, false);
        (format!("{sz_formatted} ({sz} bytes)"), None)
    } else {
        ("-".to_string(), None)
    };

    let (created_display, modified_display, accessed_display) = if let Some(ref m) = meta {
        (
            format_system_time(m.created().ok()),
            format_system_time(m.modified().ok()),
            format_system_time(m.accessed().ok()),
        )
    } else {
        ("-".to_string(), "-".to_string(), "-".to_string())
    };

    let path_w = crate::app::wide(&target_path.to_string_lossy());
    let raw_attrs = unsafe { GetFileAttributesW(path_w.as_ptr()) };
    let mut attr_list = Vec::new();
    if raw_attrs != INVALID_FILE_ATTRIBUTES {
        if (raw_attrs & FILE_ATTRIBUTE_READONLY) != 0 {
            attr_list.push("Read-only");
        }
        if (raw_attrs & FILE_ATTRIBUTE_HIDDEN) != 0 {
            attr_list.push("Hidden");
        }
        if (raw_attrs & FILE_ATTRIBUTE_SYSTEM) != 0 {
            attr_list.push("System");
        }
        if (raw_attrs & FILE_ATTRIBUTE_ARCHIVE) != 0 {
            attr_list.push("Archive");
        }
        if (raw_attrs & FILE_ATTRIBUTE_DIRECTORY) != 0 {
            attr_list.push("Directory");
        }
    }
    let attributes_display = if attr_list.is_empty() {
        "Normal".to_string()
    } else {
        attr_list.join(", ")
    };

    let file_type_desc = if is_drive {
        "Logical Drive Volume".to_string()
    } else if is_dir {
        "File Folder".to_string()
    } else if let Some(ext) = target_path.extension().and_then(|e| e.to_str()) {
        format!("{} File (.{})", ext.to_uppercase(), ext.to_lowercase())
    } else {
        "Binary File".to_string()
    };

    // Calculate SHA-256 for non-dir files under 80 MB
    let sha256 = if !is_dir {
        if let Some(ref m) = meta {
            if m.len() <= 80 * 1024 * 1024 {
                crate::app::compute_file_sha256(target_path)
            } else {
                Some("(File > 80 MB, skipped for speed)".to_string())
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut state = Box::new(PropertiesState {
        path: target_path.to_path_buf(),
        name,
        is_dir,
        is_drive,
        file_type_desc,
        location: parent_str,
        size_display,
        contains_display,
        created_display,
        modified_display,
        accessed_display,
        attributes_display,
        sha256,
        status_feedback: None,

        font_normal: crate::app::create_ui_font(-14, FW_NORMAL as i32),
        font_bold: crate::app::create_ui_font(-14, FW_SEMIBOLD as i32),
        font_title: crate::app::create_ui_font(-18, FW_SEMIBOLD as i32),
        font_small: crate::app::create_ui_font(-12, FW_NORMAL as i32),
        font_mono: crate::app::create_ui_font(-12, FW_NORMAL as i32),
    });

    unsafe {
        let class_name = crate::app::wide("RovikPropertiesDialog");
        let title = format!("Properties: {}", state.name);

        let dlg = crate::app::create_dark_dialog(
            owner,
            &class_name,
            Some(properties_wndproc),
            &title,
            DLG_W,
            DLG_H,
            state.as_mut() as *mut _ as *const _,
        );

        if dlg.is_null() {
            return;
        }

        // Action Buttons at bottom
        let btn_y = DLG_H - 46;
        let mut btn_x = 20;

        crate::app::create_child(
            dlg, "BUTTON", "Copy Path",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            btn_x, btn_y, 90, 30,
            ID_BTN_COPY_PATH, state.font_normal,
        );
        btn_x += 96;

        if state.sha256.is_some() {
            crate::app::create_child(
                dlg, "BUTTON", "Copy SHA-256",
                WS_TABSTOP | BS_OWNERDRAW as u32,
                btn_x, btn_y, 100, 30,
                ID_BTN_COPY_SHA, state.font_normal,
            );
            btn_x += 106;
        }

        crate::app::create_child(
            dlg, "BUTTON", "Windows Props",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            btn_x, btn_y, 105, 30,
            ID_BTN_SYS_PROPS, state.font_normal,
        );

        crate::app::create_child(
            dlg, "BUTTON", "Close",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            DLG_W - 88, btn_y, 68, 30,
            ID_BTN_CLOSE, state.font_bold,
        );

        crate::app::run_modal(owner, dlg);

        DeleteObject(state.font_normal);
        DeleteObject(state.font_bold);
        DeleteObject(state.font_title);
        DeleteObject(state.font_small);
        DeleteObject(state.font_mono);
    }
}

unsafe extern "system" fn properties_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let cs = lparam as *const CREATESTRUCTW;
            if !cs.is_null() && !(*cs).lpCreateParams.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            }
            0
        }
        WM_PAINT => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PropertiesState;
            if state_ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let s = &*state_ptr;

            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);

            let mut client_rc: RECT = zeroed();
            GetClientRect(hwnd, &mut client_rc);

            // Background
            crate::app::round_rect_solid(dc, &client_rc, 0, crate::app::CLR_SIDEBAR_BG, None);

            // 1. Header Card (Icon + Title + Category)
            let header_rc = RECT {
                left: 16,
                top: 14,
                right: DLG_W - 16,
                bottom: 84,
            };
            crate::app::round_rect_solid(
                dc,
                &header_rc,
                12,
                crate::app::CLR_MAIN_CARD_BG,
                Some(crate::app::CLR_CARD_BORDER),
            );

            // Vector Icon
            let icon_x = header_rc.left + 16;
            let icon_y = header_rc.top + 16;
            if s.is_drive {
                crate::app::draw_vector_drive(dc, icon_x, icon_y, crate::app::CLR_ACCENT);
            } else if s.is_dir {
                crate::app::draw_vector_folder(dc, icon_x, icon_y, 0x005CB2E2); // warm amber
            } else {
                crate::app::draw_vector_file(dc, icon_x, icon_y, 0x00F0B96C); // sky blue
            }

            // Name
            let name_rc = RECT {
                left: icon_x + 30,
                top: header_rc.top + 12,
                right: header_rc.right - 14,
                bottom: header_rc.top + 36,
            };
            crate::app::draw_text_in(
                dc,
                &s.name,
                name_rc,
                crate::app::CLR_PRIMARY_TEXT,
                s.font_title,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            // Category
            let cat_rc = RECT {
                left: icon_x + 30,
                top: header_rc.top + 38,
                right: header_rc.right - 14,
                bottom: header_rc.top + 58,
            };
            crate::app::draw_text_in(
                dc,
                &s.file_type_desc,
                cat_rc,
                crate::app::CLR_SECONDARY_TEXT,
                s.font_normal,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );

            // 2. Details Card
            let details_rc = RECT {
                left: 16,
                top: 96,
                right: DLG_W - 16,
                bottom: if s.sha256.is_some() { 360 } else { 480 },
            };
            crate::app::round_rect_solid(
                dc,
                &details_rc,
                12,
                crate::app::CLR_MAIN_CARD_BG,
                Some(crate::app::CLR_CARD_BORDER),
            );

            let mut row_y = details_rc.top + 14;
            let label_x = details_rc.left + 16;
            let val_x = details_rc.left + 104;
            let val_right = details_rc.right - 14;
            let row_h = 28;

            let fields: Vec<(&str, &str)> = {
                let mut v = Vec::new();
                v.push(("Type:", s.file_type_desc.as_str()));
                v.push(("Location:", s.location.as_str()));
                v.push(("Size:", s.size_display.as_str()));
                if let Some(ref c) = s.contains_display {
                    v.push(("Contains:", c.as_str()));
                }
                v.push(("Created:", s.created_display.as_str()));
                v.push(("Modified:", s.modified_display.as_str()));
                v.push(("Accessed:", s.accessed_display.as_str()));
                v.push(("Attributes:", s.attributes_display.as_str()));
                v
            };

            for (lbl, val) in fields {
                let lbl_rc = RECT {
                    left: label_x,
                    top: row_y,
                    right: val_x - 10,
                    bottom: row_y + 20,
                };
                crate::app::draw_text_in(
                    dc,
                    lbl,
                    lbl_rc,
                    crate::app::CLR_MUTED_TEXT,
                    s.font_bold,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );

                let val_rc = RECT {
                    left: val_x,
                    top: row_y,
                    right: val_right,
                    bottom: row_y + 20,
                };
                crate::app::draw_text_in(
                    dc,
                    val,
                    val_rc,
                    crate::app::CLR_PRIMARY_TEXT,
                    s.font_normal,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
                );

                row_y += row_h;
            }

            // 3. Checksum Card (if present)
            if let Some(ref hash) = s.sha256 {
                let sha_rc = RECT {
                    left: 16,
                    top: 372,
                    right: DLG_W - 16,
                    bottom: 480,
                };
                crate::app::round_rect_solid(
                    dc,
                    &sha_rc,
                    12,
                    crate::app::CLR_MAIN_CARD_BG,
                    Some(crate::app::CLR_CARD_BORDER),
                );

                let lbl_sha = RECT {
                    left: sha_rc.left + 16,
                    top: sha_rc.top + 12,
                    right: sha_rc.right - 16,
                    bottom: sha_rc.top + 30,
                };
                crate::app::draw_text_in(
                    dc,
                    "CHECKSUM (SHA-256)",
                    lbl_sha,
                    crate::app::CLR_MUTED_TEXT,
                    s.font_small,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );

                let hash_box = RECT {
                    left: sha_rc.left + 16,
                    top: sha_rc.top + 34,
                    right: sha_rc.right - 16,
                    bottom: sha_rc.top + 94,
                };
                crate::app::round_rect_solid(
                    dc,
                    &hash_box,
                    6,
                    crate::app::CLR_INPUT_BG,
                    Some(crate::app::CLR_CARD_BORDER),
                );

                let hash_text_rc = RECT {
                    left: hash_box.left + 8,
                    top: hash_box.top + 6,
                    right: hash_box.right - 8,
                    bottom: hash_box.bottom - 6,
                };
                crate::app::draw_text_in(
                    dc,
                    hash,
                    hash_text_rc,
                    crate::app::CLR_ACCENT,
                    s.font_mono,
                    DT_LEFT | DT_NOPREFIX | DT_WORDBREAK,
                );
            }

            // Status feedback (e.g. "Path copied to clipboard!")
            if let Some(ref fb) = s.status_feedback {
                let fb_rc = RECT {
                    left: 20,
                    top: DLG_H - 80,
                    right: DLG_W - 20,
                    bottom: DLG_H - 56,
                };
                crate::app::draw_text_in(
                    dc,
                    fb,
                    fb_rc,
                    crate::app::CLR_ACCENT,
                    s.font_bold,
                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
            }

            EndPaint(hwnd, &ps);
            0
        }
        WM_DRAWITEM => {
            let dis = lparam as *const DRAWITEMSTRUCT;
            if dis.is_null() {
                return 0;
            }
            let id = (*dis).CtlID as usize;

            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PropertiesState;
            let (font_norm, font_b) = if !state_ptr.is_null() {
                ((*state_ptr).font_normal, (*state_ptr).font_bold)
            } else {
                (GetStockObject(NULL_BRUSH as i32) as _, GetStockObject(NULL_BRUSH as i32) as _)
            };

            match id {
                ID_BTN_CLOSE => {
                    crate::app::draw_dialog_button(&*dis, crate::app::BtnKind::Primary, font_b, crate::app::CLR_SIDEBAR_BG);
                }
                ID_BTN_COPY_PATH | ID_BTN_COPY_SHA | ID_BTN_SYS_PROPS => {
                    crate::app::draw_dialog_button(&*dis, crate::app::BtnKind::Secondary, font_norm, crate::app::CLR_SIDEBAR_BG);
                }
                _ => {}
            }
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PropertiesState;
            if state_ptr.is_null() {
                return 0;
            }
            let s = &mut *state_ptr;

            match id {
                ID_BTN_COPY_PATH => {
                    let path_str = s.path.to_string_lossy().to_string();
                    crate::app::copy_to_clipboard(hwnd, &path_str);
                    s.status_feedback = Some("Path copied to clipboard!".to_string());
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                ID_BTN_COPY_SHA => {
                    if let Some(ref hash) = s.sha256 {
                        crate::app::copy_to_clipboard(hwnd, hash);
                        s.status_feedback = Some("SHA-256 hash copied to clipboard!".to_string());
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                ID_BTN_SYS_PROPS => {
                    open_windows_properties(hwnd, &s.path);
                }
                ID_BTN_CLOSE | 2 => {
                    crate::app::close_dark_dialog(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_KEYDOWN => {
            let vk = wparam as u16;
            if vk == VK_ESCAPE {
                crate::app::close_dark_dialog(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
