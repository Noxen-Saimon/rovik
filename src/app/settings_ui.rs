use crate::settings::{Editor, Settings, StartupMode, Terminal};
use std::{
    mem::zeroed,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetStockObject, InvalidateRect,
        SetBkColor, SetBkMode, SetTextColor, DT_LEFT, DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD,
        HBRUSH, HDC, HFONT, NULL_BRUSH, OPAQUE, PAINTSTRUCT,
    },
    System::Com::CoTaskMemFree,
    UI::{
        Controls::DRAWITEMSTRUCT,
        Shell::{SHBrowseForFolderW, SHGetPathFromIDListW, BROWSEINFOW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS},
        WindowsAndMessaging::*,
    },
};

const ES_AUTOHSCROLL: u32 = 0x0080;

// UI Control IDs
const ID_BTN_SAVE: usize = 1001;
const ID_BTN_CANCEL: usize = 1002;
const ID_BTN_STARTUP_MODE: usize = 1003;
const ID_EDIT_STARTUP_FOLDER: usize = 1004;
const ID_BTN_BROWSE_STARTUP: usize = 1005;
const ID_BTN_REG_DEFAULT: usize = 1006;
const ID_BTN_UNREG_DEFAULT: usize = 1007;

const ID_TOGGLE_HIDDEN: usize = 1010;
const ID_TOGGLE_EXTENSIONS: usize = 1011;
const ID_TOGGLE_FOLDERS_FIRST: usize = 1012;

const ID_TOGGLE_RECYCLE: usize = 1020;
const ID_TOGGLE_CONFIRM_DELETE: usize = 1021;
const ID_TOGGLE_WARN_EXT: usize = 1022;
const ID_TOGGLE_SELECT_STEM: usize = 1023;

const ID_BTN_EDITOR: usize = 1030;
const ID_EDIT_EDITOR_PATH: usize = 1031;
const ID_BTN_BROWSE_EDITOR: usize = 1032;
const ID_BTN_TERMINAL: usize = 1033;
const ID_TOGGLE_OPEN_TEXT_IN_EDITOR: usize = 1034;

const DLG_W: i32 = 560;
const DLG_H: i32 = 680;

struct SettingsDialogState {
    settings: Settings,
    saved: bool,
    font_normal: HFONT,
    font_bold: HFONT,
    font_title: HFONT,
    font_small: HFONT,
    brush_input: HBRUSH,
    brush_bg: HBRUSH,
}

fn browse_for_folder(owner: HWND, title: &str) -> Option<String> {
    unsafe {
        let title_w = crate::app::wide(title);
        let mut bi: BROWSEINFOW = zeroed();
        bi.hwndOwner = owner;
        bi.lpszTitle = title_w.as_ptr();
        bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;

        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }

        let mut path_buf = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, path_buf.as_mut_ptr());
        CoTaskMemFree(pidl as _);

        if ok != 0 {
            let len = path_buf.iter().position(|&c| c == 0).unwrap_or(0);
            Some(String::from_utf16_lossy(&path_buf[..len]))
        } else {
            None
        }
    }
}

fn browse_for_file(owner: HWND, filter: &str) -> Option<String> {
    unsafe {
        let mut file_buf = [0u16; 512];
        let filter_w = crate::app::wide(filter);

        let mut ofn: windows_sys::Win32::UI::Controls::Dialogs::OPENFILENAMEW = zeroed();
        ofn.lStructSize = std::mem::size_of::<windows_sys::Win32::UI::Controls::Dialogs::OPENFILENAMEW>() as u32;
        ofn.hwndOwner = owner;
        ofn.lpstrFilter = filter_w.as_ptr();
        ofn.lpstrFile = file_buf.as_mut_ptr();
        ofn.nMaxFile = file_buf.len() as u32;
        ofn.Flags = windows_sys::Win32::UI::Controls::Dialogs::OFN_FILEMUSTEXIST
            | windows_sys::Win32::UI::Controls::Dialogs::OFN_PATHMUSTEXIST
            | windows_sys::Win32::UI::Controls::Dialogs::OFN_NOCHANGEDIR;

        if windows_sys::Win32::UI::Controls::Dialogs::GetOpenFileNameW(&mut ofn) != 0 {
            let len = file_buf.iter().position(|&c| c == 0).unwrap_or(0);
            Some(String::from_utf16_lossy(&file_buf[..len]))
        } else {
            None
        }
    }
}

unsafe extern "system" fn settings_dlg_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsDialogState;

    match msg {
        WM_CREATE => {
            let cs = lparam as *const CREATESTRUCTW;
            if !cs.is_null() && !(*cs).lpCreateParams.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            }
            0
        }
        WM_ERASEBKGND => {
            if !st.is_null() {
                let mut rc: RECT = zeroed();
                GetClientRect(hwnd, &mut rc);
                FillRect(wparam as HDC, &rc, (*st).brush_bg);
                1
            } else {
                1
            }
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            if !st.is_null() {
                let s = &*st;

                // Title
                crate::app::draw_text_in(
                    dc,
                    "Preferences & Settings",
                    RECT { left: 24, top: 18, right: DLG_W - 24, bottom: 42 },
                    crate::app::CLR_PRIMARY_TEXT,
                    s.font_title,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                crate::app::draw_text_in(
                    dc,
                    "Customize file operations, view display, tools, and Windows integration.",
                    RECT { left: 24, top: 42, right: DLG_W - 24, bottom: 60 },
                    crate::app::CLR_SECONDARY_TEXT,
                    s.font_small,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );

                // Section 1: STARTUP & INTEGRATION
                crate::app::draw_text_in(
                    dc,
                    "STARTUP & INTEGRATION",
                    RECT { left: 24, top: 72, right: DLG_W - 24, bottom: 88 },
                    crate::app::CLR_ACCENT,
                    s.font_bold,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                crate::app::draw_text_in(
                    dc,
                    "Open on start:",
                    RECT { left: 26, top: 96, right: 140, bottom: 124 },
                    crate::app::CLR_PRIMARY_TEXT,
                    s.font_normal,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );

                // Section 2: VIEW OPTIONS
                crate::app::draw_text_in(
                    dc,
                    "VIEW OPTIONS",
                    RECT { left: 24, top: 200, right: DLG_W - 24, bottom: 216 },
                    crate::app::CLR_ACCENT,
                    s.font_bold,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );

                // Section 3: FILE OPERATIONS & RENAMING
                crate::app::draw_text_in(
                    dc,
                    "FILE OPERATIONS & RENAMING",
                    RECT { left: 24, top: 326, right: DLG_W - 24, bottom: 342 },
                    crate::app::CLR_ACCENT,
                    s.font_bold,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );

                // Section 4: DEFAULT APPS & TOOLS
                crate::app::draw_text_in(
                    dc,
                    "DEFAULT APPS & TOOLS",
                    RECT { left: 24, top: 486, right: DLG_W - 24, bottom: 502 },
                    crate::app::CLR_ACCENT,
                    s.font_bold,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                crate::app::draw_text_in(
                    dc,
                    "Text editor:",
                    RECT { left: 26, top: 510, right: 120, bottom: 538 },
                    crate::app::CLR_PRIMARY_TEXT,
                    s.font_normal,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                crate::app::draw_text_in(
                    dc,
                    "Terminal:",
                    RECT { left: 290, top: 510, right: 370, bottom: 538 },
                    crate::app::CLR_PRIMARY_TEXT,
                    s.font_normal,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );

                // Bottom divider line
                let sep_y = DLG_H - 68;
                crate::app::round_rect_solid(
                    dc,
                    &RECT { left: 20, top: sep_y, right: DLG_W - 20, bottom: sep_y + 1 },
                    0,
                    crate::app::CLR_CARD_BORDER,
                    None,
                );
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            if st.is_null() {
                return 0;
            }
            let s = &mut *st;

            match id {
                ID_BTN_SAVE => {
                    // Pull custom folder & editor text if currently visible
                    let edit_sf = GetDlgItem(hwnd, ID_EDIT_STARTUP_FOLDER as i32);
                    if !edit_sf.is_null() && IsWindowVisible(edit_sf) != 0 {
                        s.settings.startup_folder = crate::app::get_control_text(hwnd, ID_EDIT_STARTUP_FOLDER);
                    }
                    let edit_ed = GetDlgItem(hwnd, ID_EDIT_EDITOR_PATH as i32);
                    if !edit_ed.is_null() && IsWindowVisible(edit_ed) != 0 {
                        s.settings.editor_custom_path = crate::app::get_control_text(hwnd, ID_EDIT_EDITOR_PATH);
                    }

                    s.saved = true;
                    crate::app::close_dark_dialog(hwnd);
                }
                ID_BTN_CANCEL => {
                    s.saved = false;
                    crate::app::close_dark_dialog(hwnd);
                }
                ID_BTN_STARTUP_MODE => {
                    s.settings.startup_mode = match s.settings.startup_mode {
                        StartupMode::Home => StartupMode::LastSession,
                        StartupMode::LastSession => StartupMode::Custom,
                        StartupMode::Custom => StartupMode::Home,
                    };
                    update_control_states(hwnd, s);
                }
                ID_BTN_BROWSE_STARTUP => {
                    if let Some(folder) = browse_for_folder(hwnd, "Select Custom Startup Folder") {
                        s.settings.startup_folder = folder.clone();
                        crate::app::set_control_text(hwnd, ID_EDIT_STARTUP_FOLDER, &folder);
                    }
                }
                ID_BTN_REG_DEFAULT => {
                    crate::app::register_as_default_explorer();
                }
                ID_BTN_UNREG_DEFAULT => {
                    crate::app::unregister_default_explorer();
                }
                ID_TOGGLE_HIDDEN => {
                    s.settings.show_hidden = !s.settings.show_hidden;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_HIDDEN as i32), ptr::null(), 1);
                }
                ID_TOGGLE_EXTENSIONS => {
                    s.settings.show_extensions = !s.settings.show_extensions;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_EXTENSIONS as i32), ptr::null(), 1);
                }
                ID_TOGGLE_FOLDERS_FIRST => {
                    s.settings.folders_first = !s.settings.folders_first;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_FOLDERS_FIRST as i32), ptr::null(), 1);
                }
                ID_TOGGLE_RECYCLE => {
                    s.settings.recycle_bin = !s.settings.recycle_bin;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_RECYCLE as i32), ptr::null(), 1);
                }
                ID_TOGGLE_CONFIRM_DELETE => {
                    s.settings.confirm_delete = !s.settings.confirm_delete;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_CONFIRM_DELETE as i32), ptr::null(), 1);
                }
                ID_TOGGLE_WARN_EXT => {
                    s.settings.warn_ext_change = !s.settings.warn_ext_change;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_WARN_EXT as i32), ptr::null(), 1);
                }
                ID_TOGGLE_SELECT_STEM => {
                    s.settings.rename_select_stem = !s.settings.rename_select_stem;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_SELECT_STEM as i32), ptr::null(), 1);
                }
                ID_BTN_EDITOR => {
                    s.settings.editor = match s.settings.editor {
                        Editor::Zed => Editor::VSCodium,
                        Editor::VSCodium => Editor::Notepad,
                        Editor::Notepad => Editor::Custom,
                        Editor::Custom => Editor::Zed,
                    };
                    update_control_states(hwnd, s);
                }
                ID_BTN_BROWSE_EDITOR => {
                    if let Some(exe) = browse_for_file(hwnd, "Executable Files (*.exe)\0*.exe\0All Files (*.*)\0*.*\0\0") {
                        s.settings.editor_custom_path = exe.clone();
                        crate::app::set_control_text(hwnd, ID_EDIT_EDITOR_PATH, &exe);
                    }
                }
                ID_BTN_TERMINAL => {
                    s.settings.terminal = match s.settings.terminal {
                        Terminal::PowerShell => Terminal::WindowsTerminal,
                        Terminal::WindowsTerminal => Terminal::Cmd,
                        Terminal::Cmd => Terminal::PowerShell,
                    };
                    update_control_states(hwnd, s);
                }
                ID_TOGGLE_OPEN_TEXT_IN_EDITOR => {
                    s.settings.open_text_in_editor = !s.settings.open_text_in_editor;
                    InvalidateRect(GetDlgItem(hwnd, ID_TOGGLE_OPEN_TEXT_IN_EDITOR as i32), ptr::null(), 1);
                }
                _ => {}
            }
            0
        }
        WM_DRAWITEM => {
            if st.is_null() {
                return 1;
            }
            let s = &*st;
            let dis = &*(lparam as *const DRAWITEMSTRUCT);
            let id = dis.CtlID as usize;

            match id {
                ID_BTN_SAVE => {
                    crate::app::draw_dialog_button(dis, crate::app::BtnKind::Primary, s.font_bold, crate::app::CLR_SIDEBAR_BG);
                }
                ID_BTN_CANCEL | ID_BTN_REG_DEFAULT | ID_BTN_UNREG_DEFAULT | ID_BTN_BROWSE_STARTUP | ID_BTN_BROWSE_EDITOR => {
                    crate::app::draw_dialog_button(dis, crate::app::BtnKind::Secondary, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_BTN_STARTUP_MODE | ID_BTN_EDITOR | ID_BTN_TERMINAL => {
                    crate::app::draw_dialog_button(dis, crate::app::BtnKind::Picker, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_HIDDEN => {
                    crate::app::draw_toggle_row(dis, s.settings.show_hidden, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_EXTENSIONS => {
                    crate::app::draw_toggle_row(dis, s.settings.show_extensions, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_FOLDERS_FIRST => {
                    crate::app::draw_toggle_row(dis, s.settings.folders_first, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_RECYCLE => {
                    crate::app::draw_toggle_row(dis, s.settings.recycle_bin, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_CONFIRM_DELETE => {
                    crate::app::draw_toggle_row(dis, s.settings.confirm_delete, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_WARN_EXT => {
                    crate::app::draw_toggle_row(dis, s.settings.warn_ext_change, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_SELECT_STEM => {
                    crate::app::draw_toggle_row(dis, s.settings.rename_select_stem, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                ID_TOGGLE_OPEN_TEXT_IN_EDITOR => {
                    crate::app::draw_toggle_row(dis, s.settings.open_text_in_editor, s.font_normal, crate::app::CLR_SIDEBAR_BG);
                }
                _ => {}
            }
            1
        }
        WM_CTLCOLOREDIT => {
            let dc = wparam as HDC;
            SetTextColor(dc, crate::app::CLR_PRIMARY_TEXT);
            SetBkColor(dc, crate::app::CLR_INPUT_BG);
            SetBkMode(dc, OPAQUE as i32);
            if st.is_null() { GetStockObject(NULL_BRUSH) as LRESULT } else { (*st).brush_input as LRESULT }
        }
        WM_CLOSE => {
            if !st.is_null() {
                (*st).saved = false;
            }
            crate::app::close_dark_dialog(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn update_control_states(hwnd: HWND, s: &SettingsDialogState) {
    // 1. Startup Mode Button text
    let btn_mode = GetDlgItem(hwnd, ID_BTN_STARTUP_MODE as i32);
    let mode_label = format!("{} ", s.settings.startup_mode.label());
    SetWindowTextW(btn_mode, crate::app::wide(&mode_label).as_ptr());
    InvalidateRect(btn_mode, ptr::null(), 1);

    // Show/hide startup folder input
    let is_custom_startup = s.settings.startup_mode == StartupMode::Custom;
    let edit_sf = GetDlgItem(hwnd, ID_EDIT_STARTUP_FOLDER as i32);
    let btn_browse_sf = GetDlgItem(hwnd, ID_BTN_BROWSE_STARTUP as i32);
    let sw_startup = if is_custom_startup { SW_SHOW } else { SW_HIDE };
    ShowWindow(edit_sf, sw_startup);
    ShowWindow(btn_browse_sf, sw_startup);

    // 2. Editor Button text
    let btn_ed = GetDlgItem(hwnd, ID_BTN_EDITOR as i32);
    let ed_label = format!("{} ", s.settings.editor.label());
    SetWindowTextW(btn_ed, crate::app::wide(&ed_label).as_ptr());
    InvalidateRect(btn_ed, ptr::null(), 1);

    // Show/hide custom editor path input
    let is_custom_ed = s.settings.editor == Editor::Custom;
    let edit_ed = GetDlgItem(hwnd, ID_EDIT_EDITOR_PATH as i32);
    let btn_browse_ed = GetDlgItem(hwnd, ID_BTN_BROWSE_EDITOR as i32);
    let sw_ed = if is_custom_ed { SW_SHOW } else { SW_HIDE };
    ShowWindow(edit_ed, sw_ed);
    ShowWindow(btn_browse_ed, sw_ed);

    // 3. Terminal Button text
    let btn_term = GetDlgItem(hwnd, ID_BTN_TERMINAL as i32);
    let term_label = format!("{} ", s.settings.terminal.label());
    SetWindowTextW(btn_term, crate::app::wide(&term_label).as_ptr());
    InvalidateRect(btn_term, ptr::null(), 1);
}

pub fn prompt_settings(owner: HWND, current: &Settings) -> Option<Settings> {
    unsafe {
        let mut st = SettingsDialogState {
            settings: current.clone(),
            saved: false,
            font_normal: crate::app::create_ui_font(-14, FW_NORMAL as i32),
            font_bold: crate::app::create_ui_font(-14, FW_SEMIBOLD as i32),
            font_title: crate::app::create_ui_font(-18, FW_SEMIBOLD as i32),
            font_small: crate::app::create_ui_font(-12, FW_NORMAL as i32),
            brush_input: CreateSolidBrush(crate::app::CLR_INPUT_BG),
            brush_bg: CreateSolidBrush(crate::app::CLR_SIDEBAR_BG),
        };

        let class = crate::app::wide("RovikSettingsDialog");
        let dlg = crate::app::create_dark_dialog(
            owner,
            &class,
            Some(settings_dlg_wndproc),
            "Settings — Rovik File Explorer",
            DLG_W,
            DLG_H,
            &mut st as *mut _ as *const _,
        );

        if !dlg.is_null() {
            // Section 1 Controls: Startup
            crate::app::create_child(
                dlg,
                "BUTTON",
                st.settings.startup_mode.label(),
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                142,
                96,
                200,
                28,
                ID_BTN_STARTUP_MODE,
                st.font_normal,
            );

            let _edit_sf = crate::app::create_child(
                dlg,
                "EDIT",
                &st.settings.startup_folder,
                WS_TABSTOP | ES_AUTOHSCROLL as u32,
                142,
                128,
                330,
                26,
                ID_EDIT_STARTUP_FOLDER,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "...",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                480,
                128,
                54,
                26,
                ID_BTN_BROWSE_STARTUP,
                st.font_normal,
            );

            crate::app::create_child(
                dlg,
                "BUTTON",
                "Set as Default Windows Explorer",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                160,
                246,
                28,
                ID_BTN_REG_DEFAULT,
                st.font_normal,
            );

            crate::app::create_child(
                dlg,
                "BUTTON",
                "Restore Windows Default Explorer",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                280,
                160,
                254,
                28,
                ID_BTN_UNREG_DEFAULT,
                st.font_normal,
            );

            // Section 2 Controls: View Toggles
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Show hidden and system files (Ctrl+H)",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                222,
                DLG_W - 52,
                30,
                ID_TOGGLE_HIDDEN,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Show file extensions (.txt, .rs, .png)",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                254,
                DLG_W - 52,
                30,
                ID_TOGGLE_EXTENSIONS,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Show folders on top when sorting",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                286,
                DLG_W - 52,
                30,
                ID_TOGGLE_FOLDERS_FIRST,
                st.font_normal,
            );

            // Section 3 Controls: File Operations & Rename Toggles
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Move deleted items to Recycle Bin (off = permanent)",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                348,
                DLG_W - 52,
                30,
                ID_TOGGLE_RECYCLE,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Confirm before deleting items",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                380,
                DLG_W - 52,
                30,
                ID_TOGGLE_CONFIRM_DELETE,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Warn when changing file extension on rename",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                412,
                DLG_W - 52,
                30,
                ID_TOGGLE_WARN_EXT,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Pre-select name only (exclude extension) in rename",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                444,
                DLG_W - 52,
                30,
                ID_TOGGLE_SELECT_STEM,
                st.font_normal,
            );

            // Section 4 Controls: Apps & Tools
            crate::app::create_child(
                dlg,
                "BUTTON",
                st.settings.editor.label(),
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                114,
                510,
                160,
                28,
                ID_BTN_EDITOR,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                st.settings.terminal.label(),
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                362,
                510,
                172,
                28,
                ID_BTN_TERMINAL,
                st.font_normal,
            );

            crate::app::create_child(
                dlg,
                "EDIT",
                &st.settings.editor_custom_path,
                WS_TABSTOP | ES_AUTOHSCROLL as u32,
                114,
                542,
                358,
                26,
                ID_EDIT_EDITOR_PATH,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "...",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                480,
                542,
                54,
                26,
                ID_BTN_BROWSE_EDITOR,
                st.font_normal,
            );

            crate::app::create_child(
                dlg,
                "BUTTON",
                "Double-clicking text files opens in preferred editor",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                26,
                574,
                DLG_W - 52,
                30,
                ID_TOGGLE_OPEN_TEXT_IN_EDITOR,
                st.font_normal,
            );

            // Bottom action buttons
            let bw = 120;
            let bh = 36;
            let by = DLG_H - 52;
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Cancel",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                DLG_W - 24 - bw * 2 - 12,
                by,
                bw,
                bh,
                ID_BTN_CANCEL,
                st.font_normal,
            );
            crate::app::create_child(
                dlg,
                "BUTTON",
                "Save & Apply",
                WS_TABSTOP | crate::app::BS_OWNERDRAW_U,
                DLG_W - 24 - bw,
                by,
                bw,
                bh,
                ID_BTN_SAVE,
                st.font_bold,
            );

            update_control_states(dlg, &st);

            ShowWindow(dlg, SW_SHOW);
            crate::app::run_modal(owner, dlg);
        }

        DeleteObject(st.font_normal);
        DeleteObject(st.font_bold);
        DeleteObject(st.font_title);
        DeleteObject(st.font_small);
        DeleteObject(st.brush_input);
        DeleteObject(st.brush_bg);

        if st.saved {
            Some(st.settings)
        } else {
            None
        }
    }
}
