use std::{
    mem::zeroed,
    path::PathBuf,
    ptr,
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
        FillRect, GetStockObject, InvalidateRect, RoundRect, SelectObject,
        SetBkColor, SetBkMode, SetTextColor, UpdateWindow, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
        DEFAULT_PITCH, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
        DT_VCENTER, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT,
        NULL_BRUSH, OPAQUE, PAINTSTRUCT, PS_SOLID, TRANSPARENT,
    },
    UI::{
        Controls::DRAWITEMSTRUCT,
        Input::KeyboardAndMouse::{SetFocus, VK_ESCAPE},
        WindowsAndMessaging::*,
    },
};

use crate::app::{
    close_dark_dialog, create_child, create_dark_dialog, draw_dialog_button, set_control_text,
    wide, BtnKind, CLR_ACCENT, CLR_CARD_BORDER, CLR_INPUT_BG, CLR_MAIN_CARD_BG,
    CLR_PRIMARY_TEXT, CLR_SECONDARY_TEXT, CLR_SIDEBAR_BG, PILL_RADIUS,
};

pub const DLG_W: i32 = 620;
pub const DLG_H: i32 = 520;

// UI IDs
const ID_BTN_APPLY: usize = 3001;
const ID_BTN_CANCEL: usize = 3002;

const ID_TAB_FIND_REPLACE: usize = 3010;
const ID_TAB_PREFIX_SUFFIX: usize = 3011;
const ID_TAB_NUMBERING: usize = 3012;
const ID_TAB_CHANGE_CASE: usize = 3013;

const ID_EDIT_1: usize = 3020;
const ID_EDIT_2: usize = 3021;
const ID_EDIT_3: usize = 3022;
const ID_CHK_OPT_1: usize = 3023;
const ID_CHK_KEEP_EXT: usize = 3024;

const ES_AUTOHSCROLL: u32 = 0x0080;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RenameMode {
    FindReplace,
    PrefixSuffix,
    Numbering,
    ChangeCase,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaseOption {
    Lower,
    Upper,
    Title,
}

#[derive(Clone, Debug)]
pub struct BatchRenameItem {
    pub original_path: PathBuf,
    pub original_name: String,
    pub is_dir: bool,
    pub new_name: String,
}

struct BatchRenameState {
    items: Vec<BatchRenameItem>,
    mode: RenameMode,
    find_text: String,
    replace_text: String,
    match_case: bool,
    prefix_text: String,
    suffix_text: String,
    base_name: String,
    start_num: usize,
    padding: usize,
    case_opt: CaseOption,
    keep_ext: bool,
    scroll_pos: i32,
    has_conflict: bool,

    font_normal: HFONT,
    font_bold: HFONT,
    font_small: HFONT,
    brush_bg: HBRUSH,
    brush_card: HBRUSH,
    brush_input: HBRUSH,

    applied: bool,
    result_pairs: Vec<(PathBuf, PathBuf)>,
}

pub fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    let lower_haystack = haystack.to_lowercase();
    let lower_needle = needle.to_lowercase();
    let mut result = String::new();
    let mut last_end = 0;

    for (start, _) in lower_haystack.match_indices(&lower_needle) {
        result.push_str(&haystack[last_end..start]);
        result.push_str(replacement);
        last_end = start + needle.len();
    }
    result.push_str(&haystack[last_end..]);
    result
}

pub fn to_title_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c.is_alphanumeric() {
            if capitalize_next {
                result.extend(c.to_uppercase());
                capitalize_next = false;
            } else {
                result.extend(c.to_lowercase());
            }
        } else {
            result.push(c);
            capitalize_next = true;
        }
    }
    result
}

pub fn transform_name(
    orig_name: &str,
    is_dir: bool,
    mode: RenameMode,
    find_str: &str,
    replace_str: &str,
    match_case: bool,
    prefix_str: &str,
    suffix_str: &str,
    base_name: &str,
    start_num: usize,
    padding: usize,
    case_opt: CaseOption,
    keep_ext: bool,
    index: usize,
) -> String {
    let (stem, ext) = if !is_dir && keep_ext {
        if let Some(pos) = orig_name.rfind('.') {
            (&orig_name[..pos], &orig_name[pos..])
        } else {
            (orig_name, "")
        }
    } else {
        (orig_name, "")
    };

    let new_stem = match mode {
        RenameMode::FindReplace => {
            if find_str.is_empty() {
                stem.to_string()
            } else if match_case {
                stem.replace(find_str, replace_str)
            } else {
                replace_case_insensitive(stem, find_str, replace_str)
            }
        }
        RenameMode::PrefixSuffix => {
            format!("{}{}{}", prefix_str, stem, suffix_str)
        }
        RenameMode::Numbering => {
            let num = start_num + index;
            format!("{}{:0width$}", base_name, num, width = padding)
        }
        RenameMode::ChangeCase => match case_opt {
            CaseOption::Lower => stem.to_lowercase(),
            CaseOption::Upper => stem.to_uppercase(),
            CaseOption::Title => to_title_case(stem),
        },
    };

    format!("{}{}", new_stem, ext)
}

impl BatchRenameState {
    fn update_previews(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.has_conflict = false;

        for (i, item) in self.items.iter_mut().enumerate() {
            let n = transform_name(
                &item.original_name,
                item.is_dir,
                self.mode,
                &self.find_text,
                &self.replace_text,
                self.match_case,
                &self.prefix_text,
                &self.suffix_text,
                &self.base_name,
                self.start_num,
                self.padding,
                self.case_opt,
                self.keep_ext,
                i,
            );
            if !seen.insert(n.to_lowercase()) {
                self.has_conflict = true;
            }
            item.new_name = n;
        }
    }
}

pub fn show_batch_rename_dialog(owner: HWND, items: &[BatchRenameItem]) -> Option<Vec<(PathBuf, PathBuf)>> {
    if items.is_empty() {
        return None;
    }

    let font_name = crate::app::wide("Segoe UI Variable Text");
    let font_normal = unsafe {
        CreateFontW(
            -13, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0,
            DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_name.as_ptr(),
        )
    };
    let font_bold = unsafe {
        CreateFontW(
            -13, 0, 0, 0, FW_SEMIBOLD as i32, 0, 0, 0,
            DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_name.as_ptr(),
        )
    };
    let font_small = unsafe {
        CreateFontW(
            -11, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0,
            DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_name.as_ptr(),
        )
    };

    let brush_bg = unsafe { CreateSolidBrush(CLR_SIDEBAR_BG) };
    let brush_card = unsafe { CreateSolidBrush(CLR_MAIN_CARD_BG) };
    let brush_input = unsafe { CreateSolidBrush(CLR_INPUT_BG) };

    let mut st = Box::new(BatchRenameState {
        items: items.to_vec(),
        mode: RenameMode::FindReplace,
        find_text: String::new(),
        replace_text: String::new(),
        match_case: false,
        prefix_text: String::new(),
        suffix_text: String::new(),
        base_name: "Item_".to_string(),
        start_num: 1,
        padding: 2,
        case_opt: CaseOption::Title,
        keep_ext: true,
        scroll_pos: 0,
        has_conflict: false,

        font_normal,
        font_bold,
        font_small,
        brush_bg,
        brush_card,
        brush_input,

        applied: false,
        result_pairs: Vec::new(),
    });
    st.update_previews();

    let title = format!("Batch Rename ({} file)", items.len());
    let class_name = crate::app::wide("RovikBatchRenameDialog");

    unsafe {
        let dlg = create_dark_dialog(
            owner,
            &class_name,
            Some(dialog_wndproc),
            &title,
            DLG_W,
            DLG_H,
            &*st as *const _ as _,
        );

        // Store pointer
        SetWindowLongPtrW(dlg, GWLP_USERDATA, &mut *st as *mut BatchRenameState as isize);

        // 1. Mode Tab Buttons (Top Bar)
        let tab_w = 140;
        let tab_h = 28;
        create_child(dlg, "BUTTON", "Cari & Ganti", WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 20, 12, tab_w, tab_h, ID_TAB_FIND_REPLACE, st.font_normal);
        create_child(dlg, "BUTTON", "Awalan / Akhiran", WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 166, 12, tab_w, tab_h, ID_TAB_PREFIX_SUFFIX, st.font_normal);
        create_child(dlg, "BUTTON", "Nomor Urut", WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 312, 12, tab_w, tab_h, ID_TAB_NUMBERING, st.font_normal);
        create_child(dlg, "BUTTON", "Ubah Huruf", WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 458, 12, tab_w, tab_h, ID_TAB_CHANGE_CASE, st.font_normal);

        // 2. Input Controls (Dynamic based on mode)
        create_child(dlg, "EDIT", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL, 120, 52, 220, 24, ID_EDIT_1, st.font_normal);
        create_child(dlg, "EDIT", "", WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL, 380, 52, 218, 24, ID_EDIT_2, st.font_normal);
        create_child(dlg, "EDIT", "2", WS_CHILD | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL, 380, 82, 60, 24, ID_EDIT_3, st.font_normal);

        // Options Checkboxes
        create_child(dlg, "BUTTON", "Cocokkan Besar/Kecil", WS_CHILD | WS_VISIBLE | WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 120, 84, 180, 22, ID_CHK_OPT_1, st.font_small);
        create_child(dlg, "BUTTON", "Pertahankan Ekstensi Asli (.ext)", WS_CHILD | WS_VISIBLE | WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 380, 84, 218, 22, ID_CHK_KEEP_EXT, st.font_small);

        // 3. Bottom Action Buttons
        create_child(dlg, "BUTTON", "Terapkan Rename", WS_CHILD | WS_VISIBLE | WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 360, 474, 140, 32, ID_BTN_APPLY, st.font_bold);
        create_child(dlg, "BUTTON", "Batal", WS_CHILD | WS_VISIBLE | WS_TABSTOP | crate::app::BS_OWNERDRAW_U, 510, 474, 90, 32, ID_BTN_CANCEL, st.font_normal);

        ShowWindow(dlg, SW_SHOW);
        UpdateWindow(dlg);
        SetFocus(GetDlgItem(dlg, ID_EDIT_1 as i32));

        // Modal message loop
        let mut msg: MSG = zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) != 0 {
            if IsDialogMessageW(dlg, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if !IsWindow(dlg) != 0 {
                break;
            }
        }
    }

    if st.applied && !st.result_pairs.is_empty() {
        Some(st.result_pairs)
    } else {
        None
    }
}

unsafe fn update_mode_ui(hwnd: HWND, s: &mut BatchRenameState) {
    let ed1 = GetDlgItem(hwnd, ID_EDIT_1 as i32);
    let ed2 = GetDlgItem(hwnd, ID_EDIT_2 as i32);
    let ed3 = GetDlgItem(hwnd, ID_EDIT_3 as i32);
    let chk1 = GetDlgItem(hwnd, ID_CHK_OPT_1 as i32);

    match s.mode {
        RenameMode::FindReplace => {
            ShowWindow(ed1, SW_SHOW);
            ShowWindow(ed2, SW_SHOW);
            ShowWindow(ed3, SW_HIDE);
            ShowWindow(chk1, SW_SHOW);
            set_control_text(hwnd, ID_EDIT_1, &s.find_text);
            set_control_text(hwnd, ID_EDIT_2, &s.replace_text);
        }
        RenameMode::PrefixSuffix => {
            ShowWindow(ed1, SW_SHOW);
            ShowWindow(ed2, SW_SHOW);
            ShowWindow(ed3, SW_HIDE);
            ShowWindow(chk1, SW_HIDE);
            set_control_text(hwnd, ID_EDIT_1, &s.prefix_text);
            set_control_text(hwnd, ID_EDIT_2, &s.suffix_text);
        }
        RenameMode::Numbering => {
            ShowWindow(ed1, SW_SHOW);
            ShowWindow(ed2, SW_SHOW);
            ShowWindow(ed3, SW_SHOW);
            ShowWindow(chk1, SW_HIDE);
            set_control_text(hwnd, ID_EDIT_1, &s.base_name);
            set_control_text(hwnd, ID_EDIT_2, &s.start_num.to_string());
            set_control_text(hwnd, ID_EDIT_3, &s.padding.to_string());
        }
        RenameMode::ChangeCase => {
            ShowWindow(ed1, SW_HIDE);
            ShowWindow(ed2, SW_HIDE);
            ShowWindow(ed3, SW_HIDE);
            ShowWindow(chk1, SW_SHOW);
        }
    }

    s.update_previews();
    InvalidateRect(hwnd, ptr::null(), 1);
}

unsafe extern "system" fn dialog_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            let dc = wparam as HDC;
            let mut rc: RECT = zeroed();
            GetClientRect(hwnd, &mut rc);
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if !state_ptr.is_null() {
                FillRect(dc, &rc, (*state_ptr).brush_bg);
            }
            1
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if !state_ptr.is_null() {
                let dc = wparam as HDC;
                SetTextColor(dc, CLR_PRIMARY_TEXT);
                SetBkColor(dc, CLR_INPUT_BG);
                SetBkMode(dc, OPAQUE as i32);
                return (*state_ptr).brush_input as LRESULT;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DRAWITEM => {
            let dis = lparam as *const DRAWITEMSTRUCT;
            if dis.is_null() {
                return 0;
            }
            let id = (*dis).CtlID as usize;
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if state_ptr.is_null() {
                return 0;
            }
            let s = &*state_ptr;

            match id {
                ID_BTN_APPLY => {
                    draw_dialog_button(&*dis, BtnKind::Primary, s.font_bold, CLR_SIDEBAR_BG);
                }
                ID_BTN_CANCEL => {
                    draw_dialog_button(&*dis, BtnKind::Secondary, s.font_normal, CLR_SIDEBAR_BG);
                }
                ID_TAB_FIND_REPLACE => {
                    let active = s.mode == RenameMode::FindReplace;
                    draw_dialog_button(&*dis, if active { BtnKind::Primary } else { BtnKind::Secondary }, s.font_normal, CLR_SIDEBAR_BG);
                }
                ID_TAB_PREFIX_SUFFIX => {
                    let active = s.mode == RenameMode::PrefixSuffix;
                    draw_dialog_button(&*dis, if active { BtnKind::Primary } else { BtnKind::Secondary }, s.font_normal, CLR_SIDEBAR_BG);
                }
                ID_TAB_NUMBERING => {
                    let active = s.mode == RenameMode::Numbering;
                    draw_dialog_button(&*dis, if active { BtnKind::Primary } else { BtnKind::Secondary }, s.font_normal, CLR_SIDEBAR_BG);
                }
                ID_TAB_CHANGE_CASE => {
                    let active = s.mode == RenameMode::ChangeCase;
                    draw_dialog_button(&*dis, if active { BtnKind::Primary } else { BtnKind::Secondary }, s.font_normal, CLR_SIDEBAR_BG);
                }
                ID_CHK_OPT_1 => {
                    let (chk, label) = if s.mode == RenameMode::FindReplace {
                        (s.match_case, "Cocokkan Besar/Kecil")
                    } else if s.mode == RenameMode::ChangeCase {
                        let txt = match s.case_opt {
                            CaseOption::Lower => "Huruf: kecil",
                            CaseOption::Upper => "Huruf: BESAR",
                            CaseOption::Title => "Huruf: Tiap Kata",
                        };
                        (true, txt)
                    } else {
                        (false, "")
                    };
                    draw_checkbox(&*dis, chk, label, s.font_small);
                }
                ID_CHK_KEEP_EXT => {
                    draw_checkbox(&*dis, s.keep_ext, "Pertahankan Ekstensi (.ext)", s.font_small);
                }
                _ => {}
            }
            1
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if state_ptr.is_null() {
                EndPaint(hwnd, &ps);
                return 0;
            }
            let s = &*state_ptr;

            SetBkMode(dc, TRANSPARENT as i32);
            SelectObject(dc, s.font_normal);

            // 1. Labels for Inputs
            SetTextColor(dc, CLR_SECONDARY_TEXT);
            match s.mode {
                RenameMode::FindReplace => {
                    let l1 = wide("Cari:");
                    let mut r1 = RECT { left: 24, top: 54, right: 110, bottom: 78 };
                    DrawTextW(dc, l1.as_ptr(), (l1.len() - 1) as i32, &mut r1, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

                    let l2 = wide("Ganti:");
                    let mut r2 = RECT { left: 345, top: 54, right: 375, bottom: 78 };
                    DrawTextW(dc, l2.as_ptr(), (l2.len() - 1) as i32, &mut r2, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
                RenameMode::PrefixSuffix => {
                    let l1 = wide("Awalan:");
                    let mut r1 = RECT { left: 24, top: 54, right: 110, bottom: 78 };
                    DrawTextW(dc, l1.as_ptr(), (l1.len() - 1) as i32, &mut r1, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

                    let l2 = wide("Akhiran:");
                    let mut r2 = RECT { left: 345, top: 54, right: 375, bottom: 78 };
                    DrawTextW(dc, l2.as_ptr(), (l2.len() - 1) as i32, &mut r2, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
                RenameMode::Numbering => {
                    let l1 = wide("Nama dasar:");
                    let mut r1 = RECT { left: 24, top: 54, right: 110, bottom: 78 };
                    DrawTextW(dc, l1.as_ptr(), (l1.len() - 1) as i32, &mut r1, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

                    let l2 = wide("Mulai #:");
                    let mut r2 = RECT { left: 345, top: 54, right: 375, bottom: 78 };
                    DrawTextW(dc, l2.as_ptr(), (l2.len() - 1) as i32, &mut r2, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

                    let l3 = wide("Digit:");
                    let mut r3 = RECT { left: 345, top: 84, right: 375, bottom: 108 };
                    DrawTextW(dc, l3.as_ptr(), (l3.len() - 1) as i32, &mut r3, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
                RenameMode::ChangeCase => {
                    let l1 = wide("Pilihan kapitalisasi teks:");
                    let mut r1 = RECT { left: 24, top: 54, right: 250, bottom: 78 };
                    DrawTextW(dc, l1.as_ptr(), (l1.len() - 1) as i32, &mut r1, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
                }
            }

            // 2. Live Preview Card
            let card_rect = RECT { left: 20, top: 120, right: 600, bottom: 460 };
            SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
            let pen_border = CreatePen(PS_SOLID as i32, 1, CLR_CARD_BORDER);
            let old_pen = SelectObject(dc, pen_border);
            FillRect(dc, &card_rect, s.brush_card);
            RoundRect(dc, card_rect.left, card_rect.top, card_rect.right, card_rect.bottom, PILL_RADIUS, PILL_RADIUS);

            // Preview Table Header
            SelectObject(dc, s.font_bold);
            SetTextColor(dc, CLR_SECONDARY_TEXT);
            let th1 = wide("Nama Asli");
            let mut r_th1 = RECT { left: 36, top: 126, right: 270, bottom: 148 };
            DrawTextW(dc, th1.as_ptr(), (th1.len() - 1) as i32, &mut r_th1, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

            let th2 = wide("Nama Baru (Pratinjau)");
            let mut r_th2 = RECT { left: 320, top: 126, right: 580, bottom: 148 };
            DrawTextW(dc, th2.as_ptr(), (th2.len() - 1) as i32, &mut r_th2, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

            // Divider line below header
            let h_line_y = 150;
            windows_sys::Win32::Graphics::Gdi::MoveToEx(dc, 28, h_line_y, ptr::null_mut());
            windows_sys::Win32::Graphics::Gdi::LineTo(dc, 592, h_line_y);

            // 3. Render Preview Rows
            SelectObject(dc, s.font_normal);
            let row_h = 28;
            let max_visible = 10;
            let start_idx = s.scroll_pos as usize;
            let end_idx = (start_idx + max_visible).min(s.items.len());

            for (row_i, item_i) in (start_idx..end_idx).enumerate() {
                let item = &s.items[item_i];
                let row_y = 154 + (row_i as i32 * row_h);

                // Alternating subtle row background
                if row_i % 2 == 1 {
                    let r_bg = RECT { left: 24, top: row_y, right: 596, bottom: row_y + row_h };
                    let br_row = CreateSolidBrush(0x00231F1D);
                    FillRect(dc, &r_bg, br_row);
                    DeleteObject(br_row);
                }

                // Left: original name
                SetTextColor(dc, CLR_SECONDARY_TEXT);
                let w_orig = wide(&item.original_name);
                let mut r_orig = RECT { left: 36, top: row_y, right: 270, bottom: row_y + row_h };
                DrawTextW(dc, w_orig.as_ptr(), (w_orig.len() - 1) as i32, &mut r_orig, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);

                // Arrow
                SetTextColor(dc, CLR_ACCENT);
                let w_arr = wide("→");
                let mut r_arr = RECT { left: 285, top: row_y, right: 305, bottom: row_y + row_h };
                DrawTextW(dc, w_arr.as_ptr(), (w_arr.len() - 1) as i32, &mut r_arr, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);

                // Right: new name (accent if changed, regular if same)
                let is_changed = item.new_name != item.original_name;
                SetTextColor(dc, if is_changed { CLR_PRIMARY_TEXT } else { CLR_SECONDARY_TEXT });
                let w_new = wide(&item.new_name);
                let mut r_new = RECT { left: 320, top: row_y, right: 585, bottom: row_y + row_h };
                DrawTextW(dc, w_new.as_ptr(), (w_new.len() - 1) as i32, &mut r_new, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
            }

            // 4. Bottom Status & Conflict Warning
            let stat_y = 478;
            if s.has_conflict {
                SetTextColor(dc, 0x005555FF); // Warning Red
                let w_warn = wide("⚠ Peringatan: Terdapat duplikasi nama baru!");
                let mut r_w = RECT { left: 24, top: stat_y, right: 340, bottom: stat_y + 24 };
                DrawTextW(dc, w_warn.as_ptr(), (w_warn.len() - 1) as i32, &mut r_w, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            } else {
                SetTextColor(dc, CLR_SECONDARY_TEXT);
                let stat_txt = format!("{} file siap diubah", s.items.len());
                let w_stat = wide(&stat_txt);
                let mut r_w = RECT { left: 24, top: stat_y, right: 340, bottom: stat_y + 24 };
                DrawTextW(dc, w_stat.as_ptr(), (w_stat.len() - 1) as i32, &mut r_w, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            }

            SelectObject(dc, old_pen);
            DeleteObject(pen_border);
            EndPaint(hwnd, &ps);
            0
        }
        WM_MOUSEWHEEL => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if !state_ptr.is_null() {
                let s = &mut *state_ptr;
                let delta = (wparam >> 16) as i16;
                let max_scroll = (s.items.len().saturating_sub(10)) as i32;
                if delta > 0 {
                    s.scroll_pos = (s.scroll_pos - 2).max(0);
                } else {
                    s.scroll_pos = (s.scroll_pos + 2).min(max_scroll);
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            let code = ((wparam >> 16) & 0xffff) as usize;
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut BatchRenameState;
            if state_ptr.is_null() {
                return 0;
            }
            let s = &mut *state_ptr;

            if code == 0x0300 { // EN_CHANGE
                let mut buf = [0u16; 512];
                let len = GetWindowTextW(lparam as HWND, buf.as_mut_ptr(), 512);
                let text = String::from_utf16_lossy(&buf[..len as usize]);

                match id {
                    ID_EDIT_1 => {
                        match s.mode {
                            RenameMode::FindReplace => s.find_text = text,
                            RenameMode::PrefixSuffix => s.prefix_text = text,
                            RenameMode::Numbering => s.base_name = text,
                            RenameMode::ChangeCase => {}
                        }
                    }
                    ID_EDIT_2 => {
                        match s.mode {
                            RenameMode::FindReplace => s.replace_text = text,
                            RenameMode::PrefixSuffix => s.suffix_text = text,
                            RenameMode::Numbering => s.start_num = text.parse().unwrap_or(1),
                            RenameMode::ChangeCase => {}
                        }
                    }
                    ID_EDIT_3 => {
                        if s.mode == RenameMode::Numbering {
                            s.padding = text.parse().unwrap_or(2).clamp(1, 8);
                        }
                    }
                    _ => {}
                }
                s.update_previews();
                InvalidateRect(hwnd, ptr::null(), 0);
                return 0;
            }

            match id {
                ID_TAB_FIND_REPLACE => {
                    s.mode = RenameMode::FindReplace;
                    update_mode_ui(hwnd, s);
                }
                ID_TAB_PREFIX_SUFFIX => {
                    s.mode = RenameMode::PrefixSuffix;
                    update_mode_ui(hwnd, s);
                }
                ID_TAB_NUMBERING => {
                    s.mode = RenameMode::Numbering;
                    update_mode_ui(hwnd, s);
                }
                ID_TAB_CHANGE_CASE => {
                    s.mode = RenameMode::ChangeCase;
                    update_mode_ui(hwnd, s);
                }
                ID_CHK_OPT_1 => {
                    if s.mode == RenameMode::FindReplace {
                        s.match_case = !s.match_case;
                    } else if s.mode == RenameMode::ChangeCase {
                        s.case_opt = match s.case_opt {
                            CaseOption::Title => CaseOption::Lower,
                            CaseOption::Lower => CaseOption::Upper,
                            CaseOption::Upper => CaseOption::Title,
                        };
                    }
                    s.update_previews();
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                ID_CHK_KEEP_EXT => {
                    s.keep_ext = !s.keep_ext;
                    s.update_previews();
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                ID_BTN_APPLY => {
                    let mut pairs = Vec::new();
                    for item in &s.items {
                        if item.original_name != item.new_name {
                            if let Some(parent) = item.original_path.parent() {
                                let new_path = parent.join(&item.new_name);
                                pairs.push((item.original_path.clone(), new_path));
                            }
                        }
                    }
                    s.result_pairs = pairs;
                    s.applied = true;
                    close_dark_dialog(hwnd);
                }
                ID_BTN_CANCEL | 2 => {
                    close_dark_dialog(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_KEYDOWN => {
            let vk = wparam as u16;
            if vk == VK_ESCAPE {
                close_dark_dialog(hwnd);
                return 0;
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn draw_checkbox(dis: &DRAWITEMSTRUCT, checked: bool, text: &str, font: HFONT) {
    let dc = dis.hDC;
    let rc = dis.rcItem;

    let br_bg = CreateSolidBrush(CLR_SIDEBAR_BG);
    FillRect(dc, &rc, br_bg);
    DeleteObject(br_bg);

    // Box
    let box_size = 14;
    let box_y = rc.top + (rc.bottom - rc.top - box_size) / 2;
    let box_rc = RECT { left: rc.left, top: box_y, right: rc.left + box_size, bottom: box_y + box_size };

    let br_box = CreateSolidBrush(if checked { CLR_ACCENT } else { CLR_INPUT_BG });
    FillRect(dc, &box_rc, br_box);
    DeleteObject(br_box);

    let pen = CreatePen(PS_SOLID as i32, 1, CLR_CARD_BORDER);
    let old_p = SelectObject(dc, pen);
    SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
    RoundRect(dc, box_rc.left, box_rc.top, box_rc.right, box_rc.bottom, 4, 4);
    SelectObject(dc, old_p);
    DeleteObject(pen);

    if checked {
        SetTextColor(dc, CLR_SIDEBAR_BG);
        let check_w = wide("✓");
        let mut cr = box_rc;
        DrawTextW(dc, check_w.as_ptr(), (check_w.len() - 1) as i32, &mut cr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }

    SelectObject(dc, font);
    SetTextColor(dc, CLR_PRIMARY_TEXT);
    SetBkMode(dc, TRANSPARENT as i32);
    let mut tr = RECT { left: rc.left + box_size + 8, top: rc.top, right: rc.right, bottom: rc.bottom };
    let w_text = wide(text);
    DrawTextW(dc, w_text.as_ptr(), (w_text.len() - 1) as i32, &mut tr, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_rename_transformations() {
        // Find & Replace
        let r1 = transform_name("my_photo_2026.jpg", false, RenameMode::FindReplace, "photo", "image", false, "", "", "", 1, 2, CaseOption::Title, true, 0);
        assert_eq!(r1, "my_image_2026.jpg");

        // Prefix & Suffix
        let r2 = transform_name("document.pdf", false, RenameMode::PrefixSuffix, "", "", false, "Projek_", "_v2", "", 1, 2, CaseOption::Title, true, 0);
        assert_eq!(r2, "Projek_document_v2.pdf");

        // Numbering
        let r3 = transform_name("any.png", false, RenameMode::Numbering, "", "", false, "", "", "Photo_", 1, 3, CaseOption::Title, true, 4);
        assert_eq!(r3, "Photo_005.png");

        // Change Case (Lower)
        let r4 = transform_name("ALL_CAPS_FILE.TXT", false, RenameMode::ChangeCase, "", "", false, "", "", "", 1, 2, CaseOption::Lower, true, 0);
        assert_eq!(r4, "all_caps_file.TXT");
    }
}
