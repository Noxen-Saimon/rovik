use crate::fs::{
    engine::{self, Entry},
    names,
    nav::Navigation,
    search::{self, SearchHandle, SearchResult},
};
use crate::preview;
use crate::settings::{Editor, Settings, StartupMode, Terminal, ViewMode};
use std::collections::BTreeSet;
use std::{
    ffi::{OsStr, OsString},
    mem::zeroed,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

mod batch_rename_ui;
pub(crate) mod drag_drop;
pub(crate) mod properties_ui;
mod settings_ui;
use windows_sys::Win32::{
    Foundation::{
        COLORREF, HINSTANCE, HWND, INVALID_HANDLE_VALUE, LPARAM, LRESULT, POINT, RECT,
        WAIT_OBJECT_0, WPARAM,
    },
    Graphics::{
        Dwm::DwmSetWindowAttribute,
        Gdi::{
            BeginPaint, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC,
            CreateFontW, CreatePen, CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, Ellipse,
            EndPaint, FillRect, GetStockObject, GetTextExtentPoint32W, GetTextFaceW,
            InvalidateRect, LineTo, MoveToEx, RoundRect, SelectObject, SetBkColor, SetBkMode,
            SetTextColor, UpdateWindow, CLEARTYPE_QUALITY, DEFAULT_CHARSET, DEFAULT_PITCH,
            DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
            DT_WORDBREAK, FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT, HPEN,
            NULL_BRUSH, OPAQUE, PAINTSTRUCT, PS_NULL, PS_SOLID, SRCCOPY, TRANSPARENT,
        },
    },
    Storage::FileSystem::{
        FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, MoveFileExW,
        FILE_NOTIFY_CHANGE_DIR_NAME, FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
        FILE_NOTIFY_CHANGE_SIZE,
    },
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable,
            OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
        },
        LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryA},
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        Registry::{
            RegCloseKey, RegCreateKeyW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
            REG_SZ,
        },
        Threading::WaitForSingleObject,
    },
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Controls::{
            CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDRF_DODEFAULT, CDRF_NOTIFYITEMDRAW,
            CDRF_NOTIFYSUBITEMDRAW, DRAWITEMSTRUCT, EM_SETSEL, NMCUSTOMDRAW, NMHDR, NM_CUSTOMDRAW,
            ODS_DISABLED, ODS_FOCUS, ODS_SELECTED, WM_MOUSELEAVE,
        },
        Input::KeyboardAndMouse::{
            EnableWindow, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent,
            TME_LEAVE, TRACKMOUSEEVENT, VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE,
            VK_F1, VK_F2, VK_F3, VK_F5, VK_HOME, VK_LBUTTON, VK_LEFT, VK_MENU, VK_NEXT,
            VK_OEM_COMMA, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
        },
        Shell::{
            DragAcceptFiles, DragFinish, DragQueryFileW, DragQueryPoint, SHChangeNotify,
            SHFileOperationW, SetCurrentProcessExplicitAppUserModelID, ShellExecuteW,
            FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_COPY, FO_DELETE,
            HDROP, SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHFILEOPSTRUCTW,
        },
        WindowsAndMessaging::*,
    },
};

#[allow(dead_code)]
const BS_PUSHBUTTON: u32 = 0x00000000;
#[allow(dead_code)]
const BS_DEFPUSHBUTTON: u32 = 0x00000001;
pub(crate) const BS_OWNERDRAW_U: u32 = 0x0000000B;

// Win32 Drive & Device Event Constants
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_REMOTE: u32 = 4;
const DRIVE_CDROM: u32 = 5;
#[allow(dead_code)]
const DRIVE_RAMDISK: u32 = 6;

const DBT_DEVICEARRIVAL: usize = 0x8000;
const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
const DBT_DEVNODES_CHANGED: usize = 0x0007;

// UI IDs for input controls
const PATH_ID: usize = 101;
const SEARCH_ID: usize = 102;
const RENAME_EDIT_ID: usize = 103;

// Custom Window Messages
const WM_USER_NAVIGATE: u32 = WM_USER + 10;
const WM_USER_CANCEL_INPUT: u32 = WM_USER + 11;
const WM_USER_DIR_CHANGED: u32 = WM_USER + 12;
const WM_USER_FOLDER_SIZE_READY: u32 = WM_USER + 13;

// Unified Command & Accelerator IDs
const CMD_OPEN: usize = 301;
const CMD_OPEN_NEW_TAB: usize = 302;
const CMD_TERMINAL: usize = 303;
const CMD_SHOW_IN_EXPLORER: usize = 304;
const CMD_COPY_PATH: usize = 305;
const CMD_COPY: usize = 306;
const CMD_CUT: usize = 307;
const CMD_PASTE: usize = 308;
const CMD_DELETE: usize = 309;
const CMD_PROPERTIES: usize = 310;
const CMD_NEW_FOLDER: usize = 311;
const CMD_REFRESH: usize = 312;
const CMD_TOGGLE_HIDDEN: usize = 313;
const CMD_OPEN_NOTEPAD: usize = 314;
const CMD_OPEN_VSCODE: usize = 315;
const CMD_COPY_NAME: usize = 316;
const CMD_COPY_RELATIVE_PATH: usize = 317;
const CMD_CHECKSUM_SHA256: usize = 318;
const CMD_DUPLICATE: usize = 319;
const CMD_NEW_TEXT_FILE: usize = 320;
const CMD_NEW_MD_FILE: usize = 321;
const CMD_SORT_NAME: usize = 322;
const CMD_SORT_TYPE: usize = 323;
const CMD_SORT_SIZE: usize = 324;
const CMD_SORT_DATE: usize = 325;
const CMD_SORT_ASC: usize = 326;
const CMD_SORT_DESC: usize = 327;
const CMD_TAB_CLOSE: usize = 328;
const CMD_TAB_CLOSE_OTHERS: usize = 329;
const CMD_TAB_DUPLICATE: usize = 330;
const CMD_FOCUS_PATH: usize = 331;
const CMD_FOCUS_SEARCH: usize = 332;
const CMD_NEXT_TAB: usize = 333;
const CMD_PREV_TAB: usize = 334;
const CMD_TAB_1: usize = 335;
const CMD_TAB_2: usize = 336;
const CMD_TAB_3: usize = 337;
const CMD_TAB_4: usize = 338;
const CMD_TAB_5: usize = 339;
const CMD_RENAME: usize = 340;
const CMD_HELP_SHORTCUTS: usize = 341;
const CMD_OPEN_ZED: usize = 342;
const CMD_OPEN_CODIUM: usize = 343;
const CMD_OPEN_PREFERRED: usize = 344;
const CMD_SETTINGS: usize = 345;
const CMD_DELETE_PERMANENT: usize = 346;
const CMD_VIEW_DETAILS: usize = 350;
const CMD_VIEW_COMPACT: usize = 351;
const CMD_VIEW_GRID: usize = 352;
const CMD_VIEW_DROPDOWN: usize = 353;
const CMD_PIN_TO_QUICK_ACCESS: usize = 354;
const CMD_UNPIN_FROM_QUICK_ACCESS: usize = 355;
const CMD_SELECT_ALL: usize = 356;
const CMD_TOGGLE_EXTENSIONS: usize = 357;
const CMD_TOGGLE_FOLDERS_FIRST: usize = 358;
const CMD_ZIP_COMPRESS: usize = 360;
const CMD_ZIP_EXTRACT_HERE: usize = 361;
const CMD_ZIP_EXTRACT_FOLDER: usize = 362;
const CMD_TOGGLE_DUAL_PANE: usize = 363;
const CMD_SORT_DROPDOWN: usize = 364;
const CMD_UNDO: usize = 365;
const CMD_REDO: usize = 366;
const CMD_CREATE_SHORTCUT: usize = 367;
const CMD_OPEN_AS: usize = 368;
const CMD_TOGGLE_GROUP_DATE: usize = 369;
const CMD_CALCULATE_FOLDER_SIZE: usize = 370;
const CMD_BATCH_RENAME: usize = 371;
const CMD_OPEN_WITH_BASE: usize = 400;
const CMD_SEND_TO_BASE: usize = 450;

const CF_HDROP: u32 = 15;
const DROPEFFECT_COPY: u32 = 1;
const DROPEFFECT_MOVE: u32 = 2;

#[repr(C)]
#[allow(non_snake_case)]
struct DROPFILES {
    pFiles: u32,
    pt: POINT,
    fNC: i32,
    fWide: i32,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}

unsafe extern "system" {}

/// Mirrors `Settings::folders_first` so `Tab::sort_current` can read it without plumbing.
static FOLDERS_FIRST: AtomicBool = AtomicBool::new(true);

// Error / warning tint used for inline validation messages
const CLR_MUTED_TEXT: COLORREF = 0x0078706A;
const VK_F10: u16 = 0x79;
const VK_SPACE: u16 = 0x20;
const CLR_ERROR_TEXT: COLORREF = 0x007A7AF0; // RGB(240, 122, 122) soft coral red

// Layout Metrics
const SIDEBAR_WIDTH: i32 = 236;
const CARD_MARGIN: i32 = 10;
const CARD_RADIUS: i32 = 16;
const PILL_RADIUS: i32 = 10;
const TAB_BAR_HEIGHT: i32 = 36;
const NAV_BAR_HEIGHT: i32 = 44;
const HEADER_HEIGHT: i32 = 28;
const ROW_HEIGHT: i32 = 34;
const STATUS_HEIGHT: i32 = 26;

// Resource limits for lightweight operation
const UNDO_STACK_MAX: usize = 50;
const REDO_STACK_MAX: usize = 50;
const FOLDER_SIZE_CACHE_MAX: usize = 100;

// Refined Palette (Humanist Muted Slate & Warm Sand)
pub(crate) const CLR_OUTER_BG: COLORREF = 0x001B1918; // RGB(24, 25, 27) deep soft canvas
pub(crate) const CLR_SIDEBAR_BG: COLORREF = 0x0024201E; // RGB(30, 32, 36) soft sidebar container
pub(crate) const CLR_MAIN_CARD_BG: COLORREF = 0x002A2523; // RGB(35, 37, 42) warm elevated card
pub(crate) const CLR_CARD_BORDER: COLORREF = 0x0038322F; // RGB(47, 50, 56) soft border
pub(crate) const CLR_ROW_SELECT: COLORREF = 0x00433A35; // RGB(53, 58, 67) comfortable active pill
pub(crate) const CLR_ROW_HOVER: COLORREF = 0x0037302C; // RGB(44, 48, 55) gentle hover pill
pub(crate) const CLR_BTN_BG: COLORREF = 0x002D2725; // RGB(37, 39, 45) soft button pill
pub(crate) const CLR_PRIMARY_TEXT: COLORREF = 0x00E1E8EC; // RGB(236, 232, 225) warm sand primary
pub(crate) const CLR_SECONDARY_TEXT: COLORREF = 0x00B0A59F; // RGB(159, 165, 176) soft secondary
pub(crate) const CLR_ACCENT: COLORREF = 0x0093A97E; // RGB(126, 169, 147) calming sage mint
pub(crate) const CLR_TAB_INACTIVE: COLORREF = 0x00221E1C; // RGB(28, 30, 34)
pub(crate) const CLR_INPUT_BG: COLORREF = 0x001F1C1B; // RGB(27, 28, 31)
pub(crate) const CLR_SCROLL_TRACK: COLORREF = 0x0024201E; // RGB(30, 32, 36) soft recessed track
pub(crate) const CLR_SCROLL_THUMB: COLORREF = 0x0048403C; // RGB(60, 64, 72) smooth slate rounded pill
pub(crate) const CLR_SCROLL_HOVER: COLORREF = 0x00625650; // RGB(80, 86, 98) soft brightened hover

// Modern Categorical Colors
const CLR_ICON_FOLDER: COLORREF = 0x005CB2E2; // RGB(226, 178, 92) warm amber
const CLR_ICON_CODE: COLORREF = 0x00F0B96C; // RGB(108, 185, 240) sky blue
const CLR_ICON_DOC: COLORREF = 0x007D7DEB; // RGB(235, 125, 125) coral terracotta
const CLR_ICON_MEDIA: COLORREF = 0x00EB8CB4; // RGB(180, 140, 235) soft violet
const CLR_ICON_ARCHIVE: COLORREF = 0x00AFC35F; // RGB(95, 195, 175) soft mint
const CLR_ICON_EXE: COLORREF = 0x008CCD78; // RGB(120, 205, 140) emerald
#[allow(dead_code)]
const CLR_ICON_DRIVE: COLORREF = 0x00B29A88; // RGB(136, 154, 178) slate blue

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortColumn {
    Name,
    Type,
    Size,
    Date,
}

#[derive(Clone)]
struct SidebarEntry {
    title: String,
    path: PathBuf,
    #[allow(dead_code)]
    is_drive: bool,
    is_pinned: bool,
    total_bytes: Option<u64>,
    free_bytes: Option<u64>,
}

struct SidebarLayoutItem {
    entry_idx: usize,
    rect: RECT,
    #[allow(dead_code)]
    is_drive: bool,
}

fn compute_sidebar_layout(
    entries: &[SidebarEntry],
    height: i32,
) -> (Vec<SidebarLayoutItem>, Option<i32>) {
    let mut items = Vec::new();
    let mut cur_y = CARD_MARGIN + 80;
    let max_y = height - CARD_MARGIN - 44;
    let mut drives_header_y = None;

    for (idx, entry) in entries.iter().enumerate() {
        if entry.is_drive && drives_header_y.is_none() {
            cur_y += 10;
            drives_header_y = Some(cur_y);
            cur_y += 22;
        }

        let item_h = if entry.is_drive { 58 } else { 30 };
        if cur_y + item_h > max_y {
            break;
        }

        let rect = RECT {
            left: CARD_MARGIN + 8,
            top: cur_y,
            right: CARD_MARGIN + SIDEBAR_WIDTH - 8,
            bottom: cur_y + item_h,
        };

        items.push(SidebarLayoutItem {
            entry_idx: idx,
            rect,
            is_drive: entry.is_drive,
        });

        cur_y += item_h + 3;
    }

    (items, drives_header_y)
}

fn format_disk_size(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const TB: u64 = 1024 * GB;
    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

fn is_exe_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

#[derive(Clone)]
struct ClipboardPayload {
    paths: Vec<PathBuf>,
    is_cut: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DateGroup {
    Today = 0,
    Yesterday = 1,
    ThisWeek = 2,
    LastWeek = 3,
    ThisMonth = 4,
    LastMonth = 5,
    ThisYear = 6,
    Older = 7,
}

impl DateGroup {
    pub fn label(&self) -> &'static str {
        match self {
            DateGroup::Today => "Hari ini",
            DateGroup::Yesterday => "Kemarin",
            DateGroup::ThisWeek => "Minggu ini",
            DateGroup::LastWeek => "Minggu lalu",
            DateGroup::ThisMonth => "Bulan ini",
            DateGroup::LastMonth => "Bulan lalu",
            DateGroup::ThisYear => "Tahun ini",
            DateGroup::Older => "Lebih lama",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DisplayRow {
    Header { group: DateGroup, count: usize },
    Item { entry_idx: usize },
}

fn unpack_days_to_ymd(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn classify_filetime(ft: u64, cur_days: i64) -> DateGroup {
    if ft == 0 {
        return DateGroup::Older;
    }
    let utc_seconds = (ft / 10_000_000).saturating_sub(11_644_473_600) as i64;
    let local_seconds = utc_seconds + TIMEZONE_OFFSET_SECS;
    let item_days = local_seconds / 86_400;
    let diff = cur_days - item_days;

    if diff <= 0 {
        return DateGroup::Today;
    }
    if diff == 1 {
        return DateGroup::Yesterday;
    }

    let dow = (cur_days + 4).rem_euclid(7);
    let days_since_monday = if dow == 0 { 6 } else { dow - 1 };

    if diff <= days_since_monday {
        return DateGroup::ThisWeek;
    }
    if diff <= days_since_monday + 7 {
        return DateGroup::LastWeek;
    }

    let (cur_y, cur_m, _) = unpack_days_to_ymd(cur_days);
    let (item_y, item_m, _) = unpack_days_to_ymd(item_days);

    if item_y == cur_y && item_m == cur_m {
        return DateGroup::ThisMonth;
    }
    if (item_y == cur_y && cur_m > 1 && item_m == cur_m - 1)
        || (cur_m == 1 && item_y == cur_y - 1 && item_m == 12)
    {
        return DateGroup::LastMonth;
    }
    if item_y == cur_y {
        return DateGroup::ThisYear;
    }
    DateGroup::Older
}

#[derive(Clone)]
struct BreadcrumbItem {
    path: PathBuf,
    rect: RECT,
}

struct Tab {
    nav: Navigation,
    entries: Vec<Entry>,
    search_handle: Option<SearchHandle>,
    search_results: Vec<SearchResult>,
    search_query: String,
    selected: BTreeSet<usize>,
    focused: Option<usize>,
    anchor: Option<usize>,
    view_mode: ViewMode,
    scroll_pos: i32,
    sort_column: SortColumn,
    sort_ascending: bool,
    group_by_date: bool,
    display_rows: Vec<DisplayRow>,
    #[allow(dead_code)]
    pending_select_paths: Vec<PathBuf>,
}

/// The item the user currently has selected, resolved for both normal and search views.
struct SelectedItem {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

impl Tab {
    fn new(path: PathBuf, show_hidden: bool, view_mode: ViewMode) -> Self {
        let mut tab = Self {
            nav: Navigation::new(path),
            entries: Vec::new(),
            search_handle: None,
            search_results: Vec::new(),
            search_query: String::new(),
            selected: BTreeSet::new(),
            focused: None,
            anchor: None,
            view_mode,
            scroll_pos: 0,
            sort_column: SortColumn::Name,
            sort_ascending: true,
            group_by_date: true,
            display_rows: Vec::new(),
            pending_select_paths: Vec::new(),
        };
        tab.reload(show_hidden);
        tab
    }

    fn is_searching(&self) -> bool {
        !self.search_query.is_empty()
    }

    fn items_count(&self) -> usize {
        if self.is_searching() {
            self.search_results.len()
        } else if self.group_by_date && self.view_mode != ViewMode::Grid {
            self.display_rows.len()
        } else {
            self.entries.len()
        }
    }

    fn display_row_to_entry(&self, row_idx: usize) -> Option<usize> {
        if self.is_searching() {
            if row_idx < self.search_results.len() {
                Some(row_idx)
            } else {
                None
            }
        } else if self.group_by_date && self.view_mode != ViewMode::Grid {
            match self.display_rows.get(row_idx) {
                Some(DisplayRow::Item { entry_idx }) => Some(*entry_idx),
                _ => None,
            }
        } else {
            if row_idx < self.entries.len() {
                Some(row_idx)
            } else {
                None
            }
        }
    }

    fn entry_to_display_row(&self, entry_idx: usize) -> Option<usize> {
        if self.is_searching() {
            if entry_idx < self.search_results.len() {
                Some(entry_idx)
            } else {
                None
            }
        } else if self.group_by_date && self.view_mode != ViewMode::Grid {
            self.display_rows
                .iter()
                .position(|r| matches!(r, DisplayRow::Item { entry_idx: e } if *e == entry_idx))
        } else {
            if entry_idx < self.entries.len() {
                Some(entry_idx)
            } else {
                None
            }
        }
    }

    fn update_display_rows(&mut self, cur_days: i64) {
        self.display_rows.clear();
        if !self.group_by_date || self.entries.is_empty() {
            for i in 0..self.entries.len() {
                self.display_rows.push(DisplayRow::Item { entry_idx: i });
            }
            return;
        }

        let mut group_counts = [0usize; 8];
        for entry in &self.entries {
            let g = classify_filetime(entry.modified_time, cur_days) as usize;
            group_counts[g] += 1;
        }

        let mut current_group = None;
        for (i, entry) in self.entries.iter().enumerate() {
            let g = classify_filetime(entry.modified_time, cur_days);
            if current_group != Some(g) {
                current_group = Some(g);
                self.display_rows.push(DisplayRow::Header {
                    group: g,
                    count: group_counts[g as usize],
                });
            }
            self.display_rows.push(DisplayRow::Item { entry_idx: i });
        }
    }

    fn is_selected(&self, idx: usize) -> bool {
        self.selected.contains(&idx)
    }

    fn primary_selected(&self) -> Option<usize> {
        self.focused
            .or_else(|| self.selected.iter().next().copied())
    }

    fn select_single(&mut self, idx: usize) {
        self.selected.clear();
        self.selected.insert(idx);
        self.focused = Some(idx);
        self.anchor = Some(idx);
    }

    fn select_toggle(&mut self, idx: usize) {
        if self.selected.contains(&idx) {
            self.selected.remove(&idx);
            if self.focused == Some(idx) {
                self.focused = self.selected.iter().next().copied();
            }
        } else {
            self.selected.insert(idx);
            self.focused = Some(idx);
            self.anchor = Some(idx);
        }
    }

    fn select_range(&mut self, to_idx: usize) {
        let anchor = self.anchor.unwrap_or(to_idx);
        let start = anchor.min(to_idx);
        let end = anchor.max(to_idx);
        self.selected.clear();
        for i in start..=end {
            self.selected.insert(i);
        }
        self.focused = Some(to_idx);
    }

    fn select_all(&mut self) {
        self.selected.clear();
        let total = self.items_count();
        for i in 0..total {
            self.selected.insert(i);
        }
        if total > 0 {
            self.focused = Some(0);
        }
    }

    fn clear_selection(&mut self) {
        self.selected.clear();
        self.focused = None;
        self.anchor = None;
    }

    fn selected_items(&self) -> Vec<SelectedItem> {
        let mut items = Vec::new();
        if self.is_searching() {
            for &idx in &self.selected {
                if let Some(r) = self.search_results.get(idx) {
                    items.push(SelectedItem {
                        name: r.entry.name_lossy(),
                        path: r.full_path.clone(),
                        is_dir: r.entry.is_dir,
                    });
                }
            }
        } else {
            let cur = self.nav.current();
            for &idx in &self.selected {
                if let Some(e) = self.entries.get(idx) {
                    items.push(SelectedItem {
                        name: e.name_lossy(),
                        path: cur.join(OsString::from_wide(&e.name)),
                        is_dir: e.is_dir,
                    });
                }
            }
        }
        items
    }

    fn selected_item(&self) -> Option<SelectedItem> {
        let idx = self.primary_selected()?;
        if self.is_searching() {
            let r = self.search_results.get(idx)?;
            Some(SelectedItem {
                name: r.entry.name_lossy(),
                path: r.full_path.clone(),
                is_dir: r.entry.is_dir,
            })
        } else {
            let e = self.entries.get(idx)?;
            Some(SelectedItem {
                name: e.name_lossy(),
                path: self.nav.current().join(OsString::from_wide(&e.name)),
                is_dir: e.is_dir,
            })
        }
    }

    fn select_path(&mut self, path: &Path) {
        let target_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if self.is_searching() {
            if let Some(pos) = self.search_results.iter().position(|r| {
                r.full_path == path || r.entry.name_lossy().to_lowercase() == target_name
            }) {
                self.select_single(pos);
            }
        } else if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.name_lossy().to_lowercase() == target_name)
        {
            self.select_single(pos);
        }
    }

    fn title(&self) -> String {
        let current = self.nav.current();
        if let Some(name) = current.file_name() {
            name.to_string_lossy().to_string()
        } else {
            current.to_string_lossy().to_string()
        }
    }

    fn reload(&mut self, show_hidden: bool) {
        if let Some(h) = self.search_handle.take() {
            h.cancel();
        }

        let prev_selected_paths: Vec<PathBuf> = self
            .selected_items()
            .into_iter()
            .map(|it| it.path)
            .collect();

        match engine::enumerate(self.nav.current(), show_hidden) {
            Ok(e) => {
                self.entries = e;
                self.sort_current();
            }
            Err(_) => {
                self.entries.clear();
            }
        }

        self.clear_selection();
        for path in prev_selected_paths {
            let name_lower = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if let Some(pos) = self
                .entries
                .iter()
                .position(|e| e.name_lossy().to_lowercase() == name_lower)
            {
                self.selected.insert(pos);
                if self.focused.is_none() {
                    self.focused = Some(pos);
                    self.anchor = Some(pos);
                }
            }
        }
    }

    fn sort_current(&mut self) {
        let asc = self.sort_ascending;
        let col = self.sort_column;
        let folders_first = FOLDERS_FIRST.load(Ordering::Relaxed);
        let cur_days = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + TIMEZONE_OFFSET_SECS)
            / 86_400;
        let group_by_date = self.group_by_date;

        self.entries.sort_unstable_by(|a, b| {
            if group_by_date {
                let ga = classify_filetime(a.modified_time, cur_days);
                let gb = classify_filetime(b.modified_time, cur_days);
                if ga != gb {
                    return ga.cmp(&gb);
                }
            }
            if folders_first && a.is_dir != b.is_dir {
                return b.is_dir.cmp(&a.is_dir);
            }
            let order = match col {
                SortColumn::Name => engine::cmp_u16_ci(&a.name, &b.name),
                SortColumn::Type => {
                    let ea = a.extension_u16().unwrap_or(&[]);
                    let eb = b.extension_u16().unwrap_or(&[]);
                    engine::cmp_u16_ci(ea, eb).then_with(|| engine::cmp_u16_ci(&a.name, &b.name))
                }
                SortColumn::Size => a
                    .size
                    .cmp(&b.size)
                    .then_with(|| engine::cmp_u16_ci(&a.name, &b.name)),
                SortColumn::Date => a
                    .modified_time
                    .cmp(&b.modified_time)
                    .then_with(|| engine::cmp_u16_ci(&a.name, &b.name)),
            };
            if asc {
                order
            } else {
                order.reverse()
            }
        });

        self.update_display_rows(cur_days);
    }

    fn apply_folder_sizes(
        &mut self,
        cache: &std::collections::HashMap<PathBuf, (u64, usize, usize)>,
    ) {
        let cur = self.nav.current();
        for entry in &mut self.entries {
            if entry.is_dir {
                let p = cur.join(OsString::from_wide(&entry.name));
                if let Some(&(sz, _, _)) = cache.get(&p) {
                    entry.size = sz;
                }
            }
        }
    }
}
struct DirWatcher {
    path: PathBuf,
    cancel: Arc<AtomicBool>,
}

impl DirWatcher {
    fn new(hwnd: HWND, path: PathBuf) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_thread = cancel.clone();
        let watch_path = path.clone();
        let hwnd_raw = hwnd as isize;

        std::thread::spawn(move || {
            let hwnd = hwnd_raw as HWND;
            let path_w = wide(&watch_path.to_string_lossy());
            let handle = unsafe {
                FindFirstChangeNotificationW(
                    path_w.as_ptr(),
                    0,
                    FILE_NOTIFY_CHANGE_FILE_NAME
                        | FILE_NOTIFY_CHANGE_DIR_NAME
                        | FILE_NOTIFY_CHANGE_LAST_WRITE
                        | FILE_NOTIFY_CHANGE_SIZE,
                )
            };

            if handle == INVALID_HANDLE_VALUE || handle.is_null() {
                return;
            }

            while !cancel_thread.load(Ordering::Relaxed) {
                let wait_res = unsafe { WaitForSingleObject(handle, 300) };
                if cancel_thread.load(Ordering::Relaxed) {
                    break;
                }

                if wait_res == WAIT_OBJECT_0 {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if cancel_thread.load(Ordering::Relaxed) {
                        break;
                    }

                    unsafe {
                        PostMessageW(hwnd, WM_USER_DIR_CHANGED, 0, 0);
                        FindNextChangeNotification(handle);
                    }
                }
            }

            unsafe {
                FindCloseChangeNotification(handle);
            }
        });

        DirWatcher { path, cancel }
    }

    fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TopButton {
    Back,
    Forward,
    #[allow(dead_code)]
    Filter,
    SortDropdown,
    ViewDropdown,
}

#[derive(Clone, Debug)]
enum FileOp {
    Copy {
        dest_paths: Vec<PathBuf>,
    },
    Move {
        pairs: Vec<(PathBuf, PathBuf)>,
    },
    Rename {
        original: PathBuf,
        new_path: PathBuf,
    },
    CreateFolder {
        path: PathBuf,
    },
    CreateFile {
        path: PathBuf,
    },
    CreateShortcut {
        path: PathBuf,
    },
}

struct State {
    tabs: Vec<Tab>,
    active_tab: usize,
    sidebar_entries: Vec<SidebarEntry>,
    sidebar_hovered: Option<usize>,
    tab_hovered: Option<usize>,
    tab_close_hovered: Option<usize>,
    tab_plus_hovered: bool,
    btn_hovered: Option<TopButton>,
    hovered_row: Option<usize>,
    show_hidden: bool,
    tracking_mouse: bool,
    clipboard: Option<ClipboardPayload>,
    breadcrumbs: Vec<BreadcrumbItem>,
    hovered_breadcrumb: Option<usize>,
    is_path_editing: bool,
    dir_watcher: Option<DirWatcher>,
    scrollbar_hovered: bool,
    scrollbar_dragging: bool,
    scrollbar_drag_offset_y: i32,
    drag_start: Option<POINT>,
    drag_candidate: Option<usize>,
    drag_deferred_select: bool,
    settings: Settings,
    settings_btn_hovered: bool,
    dual_pane: bool,
    dual_active: usize,
    left_tab: usize,
    right_tab: usize,
    undo_stack: Vec<FileOp>,
    redo_stack: Vec<FileOp>,
    status_toast: Option<(String, std::time::Instant)>,
    cached_open_with: Vec<crate::fs::shell_ext::OpenWithItem>,
    cached_send_to: Vec<crate::fs::shell_ext::SendToItem>,
    folder_size_cache:
        Arc<std::sync::Mutex<std::collections::HashMap<PathBuf, (u64, usize, usize)>>>,

    // Caches for lightweight rendering
    sidebar_layout_cache: Option<(Vec<SidebarLayoutItem>, Option<i32>)>,
    sidebar_layout_height: i32,
    scrollbar_cache: Option<ScrollbarMetrics>,
    scrollbar_cache_rect: (i32, i32),
    breadcrumbs_cached_path: Option<PathBuf>,

    // GDI resources
    brush_outer: HBRUSH,
    brush_sidebar: HBRUSH,
    brush_main_card: HBRUSH,
    brush_card_border: HBRUSH,
    brush_select: HBRUSH,
    brush_hover: HBRUSH,
    brush_btn: HBRUSH,
    brush_accent: HBRUSH,
    brush_tab_inactive: HBRUSH,
    brush_input: HBRUSH,
    brush_scroll_track: HBRUSH,
    brush_scroll_thumb: HBRUSH,
    brush_scroll_hover: HBRUSH,
    pen_null: HPEN,
    pen_card_border: HPEN,
    pen_accent: HPEN,
    font_normal: HFONT,
    font_header: HFONT,
    font_small: HFONT,
    #[allow(dead_code)]
    app_icon: HICON,
}

impl State {
    fn set_toast(&mut self, msg: String) {
        self.status_toast = Some((msg, std::time::Instant::now()));
    }

    /// Pushes an undo operation, enforcing a maximum stack size for lightweight operation.
    fn push_undo(&mut self, op: FileOp) {
        self.undo_stack.push(op);
        if self.undo_stack.len() > UNDO_STACK_MAX {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Pushes a redo operation, enforcing a maximum stack size.
    fn push_redo(&mut self, op: FileOp) {
        self.redo_stack.push(op);
        if self.redo_stack.len() > REDO_STACK_MAX {
            self.redo_stack.remove(0);
        }
    }
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Validates a user-supplied path string before passing it to Windows APIs.
/// Rejects embedded null bytes, control characters, and paths that are too long.
pub(crate) fn validate_user_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 32767 {
        return false;
    }
    // Reject embedded null bytes which can truncate paths in Windows APIs
    if path.contains('\0') {
        return false;
    }
    // Reject control characters other than tab
    if path.chars().any(|c| c.is_control() && c != '\t') {
        return false;
    }
    true
}

fn select_font_family() -> &'static [u16] {
    const SEGOE_VAR: &[u16] = &[
        b'S' as u16,
        b'e' as u16,
        b'g' as u16,
        b'o' as u16,
        b'e' as u16,
        b' ' as u16,
        b'U' as u16,
        b'I' as u16,
        b' ' as u16,
        b'V' as u16,
        b'a' as u16,
        b'r' as u16,
        b'i' as u16,
        b'a' as u16,
        b'b' as u16,
        b'l' as u16,
        b'e' as u16,
        b' ' as u16,
        b'T' as u16,
        b'e' as u16,
        b'x' as u16,
        b't' as u16,
        0,
    ];
    const SEGOE: &[u16] = &[
        b'S' as u16,
        b'e' as u16,
        b'g' as u16,
        b'o' as u16,
        b'e' as u16,
        b' ' as u16,
        b'U' as u16,
        b'I' as u16,
        0,
    ];
    const PLUS_JAKARTA: &[u16] = &[
        b'P' as u16,
        b'l' as u16,
        b'u' as u16,
        b's' as u16,
        b' ' as u16,
        b'J' as u16,
        b'a' as u16,
        b'k' as u16,
        b'a' as u16,
        b'r' as u16,
        b't' as u16,
        b'a' as u16,
        b' ' as u16,
        b'S' as u16,
        b'a' as u16,
        b'n' as u16,
        b's' as u16,
        0,
    ];
    const CANDARA: &[u16] = &[
        b'C' as u16,
        b'a' as u16,
        b'n' as u16,
        b'd' as u16,
        b'a' as u16,
        b'r' as u16,
        b'a' as u16,
        0,
    ];

    unsafe {
        let dc = CreateCompatibleDC(ptr::null_mut());
        if dc.is_null() {
            return CANDARA;
        }

        for candidate in [SEGOE_VAR, SEGOE, PLUS_JAKARTA] {
            let font = CreateFontW(
                -15,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                0,
                0,
                CLEARTYPE_QUALITY as u32,
                DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
                candidate.as_ptr(),
            );
            if !font.is_null() {
                let old = SelectObject(dc, font);
                let mut face = [0u16; 64];
                let len = GetTextFaceW(dc, 64, face.as_mut_ptr());
                SelectObject(dc, old);
                DeleteObject(font);

                let prefix = &candidate[..candidate.len() - 1];
                if len as usize >= prefix.len() && &face[..prefix.len()] == prefix {
                    DeleteDC(dc);
                    return candidate;
                }
            }
        }
        DeleteDC(dc);
        CANDARA
    }
}

fn apply_modern_window_effects(hwnd: HWND) {
    unsafe {
        // DWMWA_USE_IMMERSIVE_DARK_MODE = 20
        let dark_mode: i32 = 1;
        DwmSetWindowAttribute(
            hwnd,
            20,
            &dark_mode as *const _ as *const _,
            std::mem::size_of::<i32>() as u32,
        );

        // DWMWA_WINDOW_CORNER_PREFERENCE = 33 (2 = DWMWCP_ROUND)
        let corners: u32 = 2;
        DwmSetWindowAttribute(
            hwnd,
            33,
            &corners as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        // DWMWA_BORDER_COLOR = 34
        let border: u32 = 0x00322E2B;
        DwmSetWindowAttribute(
            hwnd,
            34,
            &border as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        // DWMWA_CAPTION_COLOR = 35 (match CLR_OUTER_BG = 0x0018191B)
        let caption: u32 = 0x0018191B;
        DwmSetWindowAttribute(
            hwnd,
            35,
            &caption as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        // DWMWA_TEXT_COLOR = 36 (match CLR_PRIMARY_TEXT)
        let text: u32 = 0x00ECE8E1;
        DwmSetWindowAttribute(
            hwnd,
            36,
            &text as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        // DWMWA_SYSTEMBACKDROP_TYPE = 38 (2 = Mica Backdrop)
        let backdrop: u32 = 2;
        DwmSetWindowAttribute(
            hwnd,
            38,
            &backdrop as *const _ as *const _,
            std::mem::size_of::<u32>() as u32,
        );

        enable_native_dark_menu();
    }
}

fn enable_native_dark_menu() {
    unsafe {
        let hmod = LoadLibraryA(b"uxtheme.dll\0".as_ptr());
        if !hmod.is_null() {
            let proc = GetProcAddress(hmod, 135 as _);
            if let Some(func) = proc {
                type FnSetPreferredAppMode = unsafe extern "system" fn(i32) -> i32;
                let set_mode: FnSetPreferredAppMode = std::mem::transmute(func);
                set_mode(2);
            }
        }
    }
}

#[allow(dead_code)]
fn get_drive_display_name(letter: char) -> String {
    let drive_root = format!("{letter}:\\");
    let drive_root_w = wide(&drive_root);

    let drive_type = unsafe { GetDriveTypeW(drive_root_w.as_ptr()) };

    let mut vol_name_buf = [0u16; 260];
    let ok = unsafe {
        GetVolumeInformationW(
            drive_root_w.as_ptr(),
            vol_name_buf.as_mut_ptr(),
            vol_name_buf.len() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        )
    };

    let vol_label = if ok != 0 {
        let len = vol_name_buf.iter().position(|&c| c == 0).unwrap_or(0);
        if len > 0 {
            String::from_utf16_lossy(&vol_name_buf[..len])
                .trim()
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    match drive_type {
        DRIVE_REMOVABLE => {
            if !vol_label.is_empty() {
                format!("{vol_label} ({letter}:)")
            } else {
                format!("USB Drive ({letter}:)")
            }
        }
        DRIVE_CDROM => {
            if !vol_label.is_empty() {
                format!("{vol_label} ({letter}:)")
            } else {
                format!("CD/DVD Drive ({letter}:)")
            }
        }
        DRIVE_REMOTE => {
            if !vol_label.is_empty() {
                format!("{vol_label} ({letter}:)")
            } else {
                format!("Network Drive ({letter}:)")
            }
        }
        DRIVE_RAMDISK => {
            if !vol_label.is_empty() {
                format!("{vol_label} ({letter}:)")
            } else {
                format!("RAM Disk ({letter}:)")
            }
        }
        DRIVE_FIXED | _ => {
            if !vol_label.is_empty() {
                format!("{vol_label} ({letter}:)")
            } else {
                format!("Local Disk ({letter}:)")
            }
        }
    }
}

fn build_sidebar_entries(pinned: &[String]) -> Vec<SidebarEntry> {
    let mut entries = Vec::new();

    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        let base = PathBuf::from(user_profile);

        entries.push(SidebarEntry {
            title: "Home".to_string(),
            path: base.clone(),
            is_drive: false,
            is_pinned: false,
            total_bytes: None,
            free_bytes: None,
        });

        let desktop = base.join("Desktop");
        if desktop.exists() {
            entries.push(SidebarEntry {
                title: "Desktop".to_string(),
                path: desktop,
                is_drive: false,
                is_pinned: false,
                total_bytes: None,
                free_bytes: None,
            });
        }

        let docs = base.join("Documents");
        if docs.exists() {
            entries.push(SidebarEntry {
                title: "Documents".to_string(),
                path: docs,
                is_drive: false,
                is_pinned: false,
                total_bytes: None,
                free_bytes: None,
            });
        }

        let downloads = base.join("Downloads");
        if downloads.exists() {
            entries.push(SidebarEntry {
                title: "Downloads".to_string(),
                path: downloads,
                is_drive: false,
                is_pinned: false,
                total_bytes: None,
                free_bytes: None,
            });
        }

        let pictures = base.join("Pictures");
        if pictures.exists() {
            entries.push(SidebarEntry {
                title: "Pictures".to_string(),
                path: pictures,
                is_drive: false,
                is_pinned: false,
                total_bytes: None,
                free_bytes: None,
            });
        }

        let videos = base.join("Videos");
        if videos.exists() {
            entries.push(SidebarEntry {
                title: "Videos".to_string(),
                path: videos,
                is_drive: false,
                is_pinned: false,
                total_bytes: None,
                free_bytes: None,
            });
        }
    }

    // Custom Pinned Folders
    for pin_path in pinned {
        let p = PathBuf::from(pin_path);
        if p.exists() {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let title = if name.is_empty() {
                pin_path.clone()
            } else {
                name
            };
            entries.push(SidebarEntry {
                title,
                path: p,
                is_drive: false,
                is_pinned: true,
                total_bytes: None,
                free_bytes: None,
            });
        }
    }

    // Logical Drives
    unsafe {
        let mask = GetLogicalDrives();
        for i in 0..26 {
            if (mask & (1 << i)) != 0 {
                let letter = (b'A' + i) as char;
                let root = format!("{letter}:\\");
                let wide_root = wide(&root);
                let drive_type = GetDriveTypeW(wide_root.as_ptr());
                let mut volume_name = [0u16; 260];
                GetVolumeInformationW(
                    wide_root.as_ptr(),
                    volume_name.as_mut_ptr(),
                    volume_name.len() as u32,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                );
                let vol_str = String::from_utf16_lossy(&volume_name)
                    .trim_matches(char::from(0))
                    .to_string();

                let title = if vol_str.is_empty() {
                    let type_name = match drive_type {
                        DRIVE_REMOVABLE => "USB Drive",
                        DRIVE_FIXED => "Local Disk",
                        DRIVE_REMOTE => "Network Drive",
                        DRIVE_CDROM => "CD/DVD Drive",
                        _ => "Drive",
                    };
                    format!("{type_name} ({letter}:)")
                } else {
                    format!("{vol_str} ({letter}:)")
                };

                // Query disk free space metrics
                let mut free_avail = 0u64;
                let mut total_bytes = 0u64;
                let mut total_free = 0u64;
                let (tot, fre) = if GetDiskFreeSpaceExW(
                    wide_root.as_ptr(),
                    &mut free_avail,
                    &mut total_bytes,
                    &mut total_free,
                ) != 0
                    && total_bytes > 0
                {
                    (Some(total_bytes), Some(free_avail))
                } else {
                    (None, None)
                };

                entries.push(SidebarEntry {
                    title,
                    path: PathBuf::from(&root),
                    is_drive: true,
                    is_pinned: false,
                    total_bytes: tot,
                    free_bytes: fre,
                });
            }
        }
    }

    entries
}
unsafe fn ensure_dir_watcher(hwnd: HWND, state: &mut State) {
    let cur_path = match state.tabs.get(state.active_tab) {
        Some(t) => t.nav.current().clone(),
        None => return,
    };

    if let Some(ref w) = state.dir_watcher {
        if w.path == cur_path {
            return;
        }
        w.stop();
    }

    state.dir_watcher = Some(DirWatcher::new(hwnd, cur_path));
}

static PREV_EDIT_WNDPROC: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn edit_subclass_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_KEYDOWN => {
            let key = wparam as u16;
            let parent = GetParent(hwnd);
            let id = GetWindowLongPtrW(hwnd, GWLP_ID) as usize;
            if key == VK_RETURN {
                if id == PATH_ID {
                    SendMessageW(parent, WM_USER_NAVIGATE, 0, 0);
                    return 0;
                } else if id == SEARCH_ID {
                    SetFocus(parent);
                    return 0;
                } else if id == RENAME_EDIT_ID {
                    SendMessageW(parent, WM_COMMAND, 1, 0);
                    return 0;
                }
            } else if key == VK_ESCAPE {
                if id == RENAME_EDIT_ID {
                    SendMessageW(parent, WM_COMMAND, 2, 0);
                    return 0;
                } else {
                    SendMessageW(parent, WM_USER_CANCEL_INPUT, 0, 0);
                    return 0;
                }
            }
        }
        _ => {}
    }
    let prev_val = PREV_EDIT_WNDPROC.load(std::sync::atomic::Ordering::Relaxed);
    let prev_proc: WNDPROC = std::mem::transmute(prev_val);
    CallWindowProcW(prev_proc, hwnd, msg, wparam, lparam)
}

// ───────────────────────── Shared dark dialog helpers ─────────────────────────

fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

pub(crate) fn create_ui_font(height: i32, weight: i32) -> HFONT {
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            select_font_family().as_ptr(),
        )
    }
}

/// Creates a hidden, owned, centered popup dialog with the given *client* size.
pub(crate) unsafe fn create_dark_dialog(
    owner: HWND,
    class_name: &[u16],
    wndproc: WNDPROC,
    title: &str,
    client_w: i32,
    client_h: i32,
    param: *const core::ffi::c_void,
) -> HWND {
    let instance = GetModuleHandleW(ptr::null());
    let mut wc: WNDCLASSW = zeroed();
    wc.style = CS_HREDRAW | CS_VREDRAW;
    wc.lpfnWndProc = wndproc;
    wc.hInstance = instance;
    wc.hCursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
    // No class brush: the background is painted in WM_ERASEBKGND. (The old code
    // gave the class a brush and then deleted it, so every later dialog painted garbage.)
    wc.hbrBackground = ptr::null_mut();
    wc.lpszClassName = class_name.as_ptr();
    RegisterClassW(&wc); // fails harmlessly when already registered

    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
    let ex_style = WS_EX_DLGMODALFRAME;
    let mut rc = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    AdjustWindowRectEx(&mut rc, style, 0, ex_style);
    let w = rc.right - rc.left;
    let h = rc.bottom - rc.top;

    let mut pr: RECT = zeroed();
    GetWindowRect(owner, &mut pr);
    let x = pr.left + ((pr.right - pr.left) - w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - h) / 2;

    let hwnd = CreateWindowExW(
        ex_style,
        class_name.as_ptr(),
        wide(title).as_ptr(),
        style,
        x,
        y,
        w,
        h,
        owner,
        ptr::null_mut(),
        instance,
        param,
    );
    if !hwnd.is_null() {
        apply_modern_window_effects(hwnd);
    }
    hwnd
}

pub(crate) unsafe fn create_child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: usize,
    font: HFONT,
) -> HWND {
    let hwnd = CreateWindowExW(
        0,
        wide(class).as_ptr(),
        wide(text).as_ptr(),
        WS_CHILD | WS_VISIBLE | style,
        x,
        y,
        w,
        h,
        parent,
        id as HMENU,
        GetModuleHandleW(ptr::null()),
        ptr::null(),
    );
    SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    hwnd
}

/// Re-enables the owner *before* destroying the dialog so Windows hands
/// activation back to Rovik instead of jumping to some other application.
pub(crate) unsafe fn close_dark_dialog(hwnd: HWND) {
    let owner = GetWindow(hwnd, GW_OWNER);
    if !owner.is_null() {
        EnableWindow(owner, 1);
    }
    DestroyWindow(hwnd);
}

fn is_edit_control(hwnd: HWND) -> bool {
    let mut buf = [0u16; 16];
    let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case("edit")
}

/// Runs a modal loop until `dlg` is destroyed; the owner is disabled meanwhile.
pub(crate) unsafe fn run_modal(owner: HWND, dlg: HWND) {
    EnableWindow(owner, 0);
    ShowWindow(dlg, SW_SHOW);
    let mut msg: MSG = zeroed();
    while IsWindow(dlg) != 0 {
        let r = GetMessageW(&mut msg, ptr::null_mut(), 0, 0);
        if r == 0 {
            // Don't swallow WM_QUIT: re-post it for the main loop.
            PostQuitMessage(msg.wParam as i32);
            break;
        }
        if r == -1 {
            break;
        }
        // Classic EDIT controls have no Ctrl+A; add it.
        if msg.message == WM_KEYDOWN
            && msg.wParam == b'A' as usize
            && GetKeyState(VK_CONTROL as i32) < 0
            && is_edit_control(msg.hwnd)
        {
            SendMessageW(msg.hwnd, EM_SETSEL, 0, -1);
            continue;
        }
        if IsDialogMessageW(dlg, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    EnableWindow(owner, 1);
    SetFocus(owner);
}

unsafe fn fill_solid(dc: HDC, rc: &RECT, color: COLORREF) {
    let b = CreateSolidBrush(color);
    FillRect(dc, rc, b);
    DeleteObject(b);
}

pub(crate) unsafe fn round_rect_solid(
    dc: HDC,
    rc: &RECT,
    radius: i32,
    fill: COLORREF,
    border: Option<COLORREF>,
) {
    let brush = CreateSolidBrush(fill);
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, c),
        None => CreatePen(PS_NULL, 0, 0),
    };
    let old_b = SelectObject(dc, brush);
    let old_p = SelectObject(dc, pen);
    RoundRect(dc, rc.left, rc.top, rc.right, rc.bottom, radius, radius);
    SelectObject(dc, old_b);
    SelectObject(dc, old_p);
    DeleteObject(brush);
    DeleteObject(pen);
}

pub(crate) unsafe fn draw_text_in(
    dc: HDC,
    text: &str,
    rc: RECT,
    color: COLORREF,
    font: HFONT,
    flags: u32,
) {
    let mut r = rc;
    let w = wide(text);
    SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    DrawTextW(
        dc,
        w.as_ptr(),
        (w.len() - 1) as i32,
        &mut r,
        flags | DT_NOPREFIX,
    );
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BtnKind {
    Primary,
    Secondary,
    Picker,
}

/// Paints an owner-drawn pill button that matches Rovik's dark theme.
pub(crate) unsafe fn draw_dialog_button(
    dis: &DRAWITEMSTRUCT,
    kind: BtnKind,
    font: HFONT,
    parent_bg: COLORREF,
) {
    let dc = dis.hDC;
    let r = dis.rcItem;
    let pressed = dis.itemState & ODS_SELECTED != 0;
    let focused = dis.itemState & ODS_FOCUS != 0;
    let disabled = dis.itemState & ODS_DISABLED != 0;

    fill_solid(dc, &r, parent_bg);

    let (fill, mut text_clr) = match kind {
        BtnKind::Primary => (if pressed { 0x007C9168 } else { CLR_ACCENT }, CLR_OUTER_BG),
        _ => (
            if pressed { CLR_ROW_SELECT } else { CLR_BTN_BG },
            CLR_PRIMARY_TEXT,
        ),
    };
    if disabled {
        text_clr = CLR_SECONDARY_TEXT;
    }
    let border = if focused || kind == BtnKind::Primary {
        CLR_ACCENT
    } else {
        CLR_CARD_BORDER
    };
    round_rect_solid(
        dc,
        &r,
        10,
        if disabled { CLR_TAB_INACTIVE } else { fill },
        Some(border),
    );

    let len = GetWindowTextLengthW(dis.hwndItem).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    GetWindowTextW(dis.hwndItem, buf.as_mut_ptr(), buf.len() as i32);
    let text = String::from_utf16_lossy(&buf[..len]);

    if kind == BtnKind::Picker {
        let tr = RECT {
            left: r.left + 12,
            top: r.top,
            right: r.right - 30,
            bottom: r.bottom,
        };
        draw_text_in(
            dc,
            &text,
            tr,
            text_clr,
            font,
            DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
        );
        // Chevron
        let cx = r.right - 16;
        let cy = (r.top + r.bottom) / 2;
        let pen = CreatePen(PS_SOLID, 2, CLR_SECONDARY_TEXT);
        let old = SelectObject(dc, pen);
        MoveToEx(dc, cx - 4, cy - 2, ptr::null_mut());
        LineTo(dc, cx, cy + 2);
        LineTo(dc, cx + 5, cy - 3);
        SelectObject(dc, old);
        DeleteObject(pen);
    } else {
        draw_text_in(
            dc,
            &text,
            r,
            text_clr,
            font,
            DT_CENTER | DT_SINGLELINE | DT_VCENTER,
        );
    }
}

/// Paints an owner-drawn toggle row: label on the left, switch on the right.
pub(crate) unsafe fn draw_toggle_row(
    dis: &DRAWITEMSTRUCT,
    on: bool,
    font: HFONT,
    parent_bg: COLORREF,
) {
    let dc = dis.hDC;
    let r = dis.rcItem;
    let focused = dis.itemState & ODS_FOCUS != 0;
    let pressed = dis.itemState & ODS_SELECTED != 0;

    fill_solid(dc, &r, parent_bg);
    if focused || pressed {
        round_rect_solid(dc, &r, 10, CLR_ROW_HOVER, None);
    }

    let len = GetWindowTextLengthW(dis.hwndItem).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    GetWindowTextW(dis.hwndItem, buf.as_mut_ptr(), buf.len() as i32);
    let text = String::from_utf16_lossy(&buf[..len]);
    let tr = RECT {
        left: r.left + 10,
        top: r.top,
        right: r.right - 58,
        bottom: r.bottom,
    };
    draw_text_in(
        dc,
        &text,
        tr,
        CLR_PRIMARY_TEXT,
        font,
        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
    );

    // Switch track + knob
    let sw = 36;
    let sh = 20;
    let sx = r.right - 10 - sw;
    let sy = (r.top + r.bottom) / 2 - sh / 2;
    let track = RECT {
        left: sx,
        top: sy,
        right: sx + sw,
        bottom: sy + sh,
    };
    round_rect_solid(
        dc,
        &track,
        sh,
        if on { CLR_ACCENT } else { CLR_SCROLL_THUMB },
        None,
    );

    let kd = 14;
    let kx = if on { sx + sw - 3 - kd } else { sx + 3 };
    let ky = sy + 3;
    let brush = CreateSolidBrush(if on { CLR_OUTER_BG } else { CLR_PRIMARY_TEXT });
    let pen = CreatePen(PS_NULL, 0, 0);
    let ob = SelectObject(dc, brush);
    let op = SelectObject(dc, pen);
    Ellipse(dc, kx, ky, kx + kd, ky + kd);
    SelectObject(dc, ob);
    SelectObject(dc, op);
    DeleteObject(brush);
    DeleteObject(pen);
}

// ───────────────────────────── Rename dialog ─────────────────────────────

const RN_W: i32 = 420;
const RN_H: i32 = 164;
const EM_LIMITTEXT: u32 = 0x00C5;

struct RenameDialogState {
    parent_dir: PathBuf,
    old_name: String,
    is_dir: bool,
    select_stem: bool,
    warn_ext_change: bool,
    error: String,
    result: Option<String>,
    font: HFONT,
    font_title: HFONT,
    brush_input: HBRUSH,
}

/// Validates the typed name. Returns `true` when the dialog may close.
unsafe fn try_accept_rename(hwnd: HWND, s: &mut RenameDialogState) -> bool {
    let edit = GetDlgItem(hwnd, RENAME_EDIT_ID as i32);
    let len = GetWindowTextLengthW(edit).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    GetWindowTextW(edit, buf.as_mut_ptr(), buf.len() as i32);
    let new_name = String::from_utf16_lossy(&buf[..len]).trim().to_string();

    let fail = |s: &mut RenameDialogState, msg: String| {
        s.error = msg;
        InvalidateRect(hwnd, ptr::null(), 1);
        SetFocus(edit);
        false
    };

    if new_name == s.old_name {
        s.result = None; // unchanged: just close
        return true;
    }
    if let Err(e) = names::validate_file_name(&new_name) {
        return fail(s, e.message());
    }
    // std::fs::rename would silently REPLACE an existing file, so refuse up front.
    // (A case-only change such as "readme.md" -> "README.md" is the same file and is allowed.)
    let target = s.parent_dir.join(&new_name);
    if !names::same_name_ci(&new_name, &s.old_name) && std::fs::symlink_metadata(&target).is_ok() {
        return fail(
            s,
            format!("An item named \"{new_name}\" already exists here."),
        );
    }
    if !s.is_dir && s.warn_ext_change && names::extension_changed(&s.old_name, &new_name) {
        let r = MessageBoxW(
            hwnd,
            wide("If you change a file name extension, the file might become unusable.\n\nAre you sure you want to change it?").as_ptr(),
            wide("Rename").as_ptr(),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        );
        if r != IDYES {
            SetFocus(edit);
            return false;
        }
    }
    s.result = Some(new_name);
    true
}

unsafe extern "system" fn rename_dlg_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut RenameDialogState;
    match msg {
        WM_CREATE => {
            let cs = lparam as *const CREATESTRUCTW;
            if !cs.is_null() && !(*cs).lpCreateParams.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            }
            0
        }
        WM_ERASEBKGND => {
            let mut rc: RECT = zeroed();
            GetClientRect(hwnd, &mut rc);
            fill_solid(wparam as HDC, &rc, CLR_SIDEBAR_BG);
            1
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            if !st.is_null() {
                let s = &*st;
                let title = if s.is_dir {
                    "Enter a new name for this folder"
                } else {
                    "Enter a new name for this file"
                };
                draw_text_in(
                    dc,
                    title,
                    RECT {
                        left: 20,
                        top: 14,
                        right: RN_W - 20,
                        bottom: 38,
                    },
                    CLR_PRIMARY_TEXT,
                    s.font_title,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER,
                );
                round_rect_solid(
                    dc,
                    &RECT {
                        left: 20,
                        top: 46,
                        right: RN_W - 20,
                        bottom: 80,
                    },
                    10,
                    CLR_INPUT_BG,
                    Some(CLR_ACCENT),
                );
                let (hint, color) = if s.error.is_empty() {
                    (
                        "Enter to rename  •  Esc to cancel".to_string(),
                        CLR_SECONDARY_TEXT,
                    )
                } else {
                    (s.error.clone(), CLR_ERROR_TEXT)
                };
                draw_text_in(
                    dc,
                    &hint,
                    RECT {
                        left: 22,
                        top: 84,
                        right: RN_W - 20,
                        bottom: 106,
                    },
                    color,
                    s.font,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                );
            }
            EndPaint(hwnd, &ps);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as usize;
            let code = ((wparam >> 16) & 0xffff) as u32;
            if id == RENAME_EDIT_ID {
                if code == EN_CHANGE && !st.is_null() && !(&(*st).error).is_empty() {
                    (*st).error.clear();
                    InvalidateRect(hwnd, ptr::null(), 1);
                }
                return 0;
            }
            if id == 1 {
                if !st.is_null() && try_accept_rename(hwnd, &mut *st) {
                    close_dark_dialog(hwnd);
                }
            } else if id == 2 {
                if !st.is_null() {
                    (*st).result = None;
                }
                close_dark_dialog(hwnd);
            }
            0
        }
        WM_DRAWITEM => {
            if !st.is_null() {
                let dis = &*(lparam as *const DRAWITEMSTRUCT);
                let kind = if dis.CtlID == 1 {
                    BtnKind::Primary
                } else {
                    BtnKind::Secondary
                };
                draw_dialog_button(dis, kind, (*st).font, CLR_SIDEBAR_BG);
            }
            1
        }
        WM_CTLCOLOREDIT => {
            // Opaque background with a real brush. The old handler returned NULL_BRUSH
            // with a transparent mode, so typed/deleted characters smeared over each other.
            let dc = wparam as HDC;
            SetTextColor(dc, CLR_PRIMARY_TEXT);
            SetBkColor(dc, CLR_INPUT_BG);
            SetBkMode(dc, OPAQUE as i32);
            if st.is_null() {
                GetStockObject(NULL_BRUSH) as LRESULT
            } else {
                (*st).brush_input as LRESULT
            }
        }
        WM_CLOSE => {
            if !st.is_null() {
                (*st).result = None;
            }
            close_dark_dialog(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Shows the rename dialog. Returns a validated new name, or `None` if cancelled/unchanged.
pub(crate) fn prompt_rename(
    owner: HWND,
    parent_dir: &Path,
    old_name: &str,
    is_dir: bool,
    select_stem: bool,
    warn_ext_change: bool,
) -> Option<String> {
    unsafe {
        let mut st = RenameDialogState {
            parent_dir: parent_dir.to_path_buf(),
            old_name: old_name.to_string(),
            is_dir,
            select_stem,
            warn_ext_change,
            error: String::new(),
            result: None,
            font: create_ui_font(-14, FW_NORMAL as i32),
            font_title: create_ui_font(-15, FW_SEMIBOLD as i32),
            brush_input: CreateSolidBrush(CLR_INPUT_BG),
        };

        let class = wide("RovikRenameDialog");
        let title = if is_dir {
            "Rename Folder"
        } else {
            "Rename File"
        };
        let dlg = create_dark_dialog(
            owner,
            &class,
            Some(rename_dlg_wndproc),
            title,
            RN_W,
            RN_H,
            &mut st as *mut _ as *const _,
        );

        if !dlg.is_null() {
            let edit = create_child(
                dlg,
                "EDIT",
                old_name,
                WS_TABSTOP | ES_AUTOHSCROLL as u32,
                32,
                54,
                RN_W - 64,
                20,
                RENAME_EDIT_ID,
                st.font,
            );
            SendMessageW(edit, EM_LIMITTEXT, 255, 0);
            // Like Explorer: pre-select only the name part (not ".ext") for files.
            let sel_end = if st.select_stem && !is_dir {
                names::stem_len_utf16(old_name)
            } else {
                old_name.encode_utf16().count()
            };
            SendMessageW(edit, EM_SETSEL, 0, sel_end as LPARAM);

            let bw = 100;
            let bh = 34;
            let by = RN_H - 18 - bh;
            create_child(
                dlg,
                "BUTTON",
                "Cancel",
                WS_TABSTOP | BS_OWNERDRAW_U,
                RN_W - 20 - bw * 2 - 10,
                by,
                bw,
                bh,
                2,
                st.font,
            );
            create_child(
                dlg,
                "BUTTON",
                "Rename",
                WS_TABSTOP | BS_OWNERDRAW_U,
                RN_W - 20 - bw,
                by,
                bw,
                bh,
                1,
                st.font,
            );

            ShowWindow(dlg, SW_SHOW);
            SetFocus(edit);
            run_modal(owner, dlg);
        }

        DeleteObject(st.font);
        DeleteObject(st.font_title);
        DeleteObject(st.brush_input);
        st.result
    }
}

/// Renames without ever overwriting an existing item, returning a friendly error.
pub(crate) fn rename_path(old: &Path, new: &Path) -> Result<(), String> {
    let old_w = wide_os(old.as_os_str());
    let new_w = wide_os(new.as_os_str());
    // Flags = 0: no MOVEFILE_REPLACE_EXISTING (std::fs::rename sets it and would clobber files).
    if unsafe { MoveFileExW(old_w.as_ptr(), new_w.as_ptr(), 0) } != 0 {
        return Ok(());
    }
    let err = std::io::Error::last_os_error();
    Err(match err.raw_os_error().unwrap_or(0) {
        2 | 3 => "The item no longer exists. It may have been moved or deleted.".into(),
        5 => "Access denied. You may need administrator permission to rename this item.".into(),
        32 | 33 => "The item is open in another program. Close it and try again.".into(),
        80 | 183 => "An item with that name already exists.".into(),
        123 => "The name contains characters that are not allowed.".into(),
        206 => "The resulting path is too long.".into(),
        19 => "The drive is write-protected.".into(),
        _ => format!("Could not rename the item.\n\n{err}"),
    })
}

fn create_edit(parent: HWND, id: usize, font: HFONT, cue: &str) -> HWND {
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            ptr::null(),
            WS_CHILD | WS_VISIBLE | ES_AUTOHSCROLL as u32,
            0,
            0,
            0,
            0,
            parent,
            id as HMENU,
            GetModuleHandleW(ptr::null()),
            ptr::null(),
        );
        SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
        let cue_w = wide(cue);
        SendMessageW(hwnd, 0x1501, 1, cue_w.as_ptr() as LPARAM);

        let prev = SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            edit_subclass_wndproc as *const () as usize as isize,
        );
        let _ = PREV_EDIT_WNDPROC.compare_exchange(
            0,
            prev as usize,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
        hwnd
    }
}

pub(crate) fn get_control_text(hwnd: HWND, id: usize) -> String {
    unsafe {
        let ctrl = GetDlgItem(hwnd, id as i32);
        let len = GetWindowTextLengthW(ctrl).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        GetWindowTextW(ctrl, buf.as_mut_ptr(), buf.len() as i32);
        String::from_utf16_lossy(&buf[..len])
    }
}

pub(crate) fn set_control_text(hwnd: HWND, id: usize, text: &str) {
    unsafe {
        let ctrl = GetDlgItem(hwnd, id as i32);
        let w = wide(text);
        SetWindowTextW(ctrl, w.as_ptr());
    }
}

pub(crate) fn copy_to_clipboard(hwnd: HWND, text: &str) {
    unsafe {
        if OpenClipboard(hwnd) == 0 {
            return;
        }
        EmptyClipboard();
        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes_len = utf16.len() * std::mem::size_of::<u16>();
        let hmem = GlobalAlloc(GMEM_MOVEABLE, bytes_len);
        if !hmem.is_null() {
            let ptr = GlobalLock(hmem) as *mut u16;
            if !ptr.is_null() {
                std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
                GlobalUnlock(hmem);
                SetClipboardData(13, hmem);
            }
        }
        CloseClipboard();
    }
}

fn open_terminal_in_dir(hwnd: HWND, state: &State, dir: &Path) {
    unsafe {
        let dir_w = wide(&dir.to_string_lossy());
        let open_w = wide("open");
        let (exe_w, args_w) = match state.settings.terminal {
            Terminal::PowerShell => (wide("powershell.exe"), wide("-NoExit")),
            Terminal::WindowsTerminal => (
                wide("wt.exe"),
                wide(&format!("-d \"{}\"", dir.to_string_lossy())),
            ),
            Terminal::Cmd => (wide("cmd.exe"), wide("/K")),
        };

        ShellExecuteW(
            hwnd,
            open_w.as_ptr(),
            exe_w.as_ptr(),
            args_w.as_ptr(),
            dir_w.as_ptr(),
            SW_SHOWNORMAL,
        );
    }
}

fn open_with_preferred_editor(hwnd: HWND, state: &State, path: &Path) {
    match state.settings.editor {
        Editor::Zed => open_with_zed(hwnd, path),
        Editor::VSCodium => open_with_codium(hwnd, path),
        Editor::Notepad => open_with_notepad(hwnd, path),
        Editor::Custom => {
            let custom = &state.settings.editor_custom_path;
            if !custom.is_empty() && Path::new(custom).is_file() {
                let exe = wide(custom);
                let arg = wide(&format!("\"{}\"", path.to_string_lossy()));
                unsafe {
                    ShellExecuteW(
                        hwnd,
                        wide("open").as_ptr(),
                        exe.as_ptr(),
                        arg.as_ptr(),
                        ptr::null(),
                        SW_SHOWNORMAL,
                    );
                }
            } else {
                open_with_zed(hwnd, path);
            }
        }
    }
}

fn show_in_explorer(hwnd: HWND, path: &Path) {
    unsafe {
        let open_w = wide("open");
        let exp_w = wide("explorer.exe");
        let param = format!("/select,\"{}\"", path.to_string_lossy());
        let param_w = wide(&param);
        ShellExecuteW(
            hwnd,
            open_w.as_ptr(),
            exp_w.as_ptr(),
            param_w.as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn show_properties(hwnd: HWND, path: &Path) {
    properties_ui::show_properties_dialog(hwnd, path);
}

fn create_new_folder(current_dir: &Path) -> Option<PathBuf> {
    let mut target = current_dir.join("New Folder");
    let mut counter = 2;
    while target.exists() {
        target = current_dir.join(format!("New Folder ({counter})"));
        counter += 1;
    }
    if std::fs::create_dir(&target).is_ok() {
        Some(target)
    } else {
        None
    }
}

fn delete_entry(path: &Path, is_dir: bool, recycle_bin: bool) -> bool {
    if recycle_bin {
        let mut path_w = wide_os(path.as_os_str());
        path_w.push(0); // double-null terminated for SHFILEOPSTRUCTW
        let mut op: SHFILEOPSTRUCTW = unsafe { zeroed() };
        op.wFunc = FO_DELETE;
        op.pFrom = path_w.as_ptr();
        op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16;
        let ret = unsafe { SHFileOperationW(&mut op) };
        if ret == 0 && op.fAnyOperationsAborted == 0 {
            return true;
        }
    }
    if is_dir {
        std::fs::remove_dir_all(path).is_ok()
    } else {
        std::fs::remove_file(path).is_ok()
    }
}

unsafe fn update_controls_for_active_tab(hwnd: HWND, state: &State) {
    if let Some(tab) = state.tabs.get(state.active_tab) {
        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
        set_control_text(hwnd, SEARCH_ID, &tab.search_query);
    }
}

struct ScrollbarMetrics {
    track_left: i32,
    track_top: i32,
    track_right: i32,
    track_bottom: i32,
    thumb_top: i32,
    thumb_bottom: i32,
    thumb_h: i32,
    max_scroll: i32,
    page_items: usize,
    needed: bool,
}

fn calc_pane_scrollbar_metrics(pane_rect: RECT, tab: &Tab) -> ScrollbarMetrics {
    let header_h = match tab.view_mode {
        ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
        ViewMode::Grid => 0,
    };
    let list_top = pane_rect.top + header_h + 2;
    let list_bottom = pane_rect.bottom;
    let list_height = (list_bottom - list_top).max(0);

    let total_count = tab.items_count();

    let (page_items, max_scroll) = match tab.view_mode {
        ViewMode::Details => {
            let row_h = 34;
            let pi = (list_height / row_h).max(1) as usize;
            let ms = (total_count.saturating_sub(pi)) as i32;
            (pi, ms)
        }
        ViewMode::Compact => {
            let row_h = 26;
            let pi = (list_height / row_h).max(1) as usize;
            let ms = (total_count.saturating_sub(pi)) as i32;
            (pi, ms)
        }
        ViewMode::Grid => {
            let p_w = (pane_rect.right - pane_rect.left - 24).max(120);
            let cols = (p_w / 116).max(1) as usize;
            let card_h = 100;
            let page_rows = (list_height / card_h).max(1) as usize;
            let pi = page_rows * cols;
            let total_rows = if total_count == 0 {
                0
            } else {
                (total_count + cols - 1) / cols
            };
            let ms = (total_rows.saturating_sub(page_rows)) as i32;
            (pi, ms)
        }
    };

    let track_w = 6;
    let track_right = pane_rect.right - 8;
    let track_left = track_right - track_w;
    let track_top = list_top + 4;
    let track_bottom = (list_bottom - 4).max(track_top);
    let track_height = (track_bottom - track_top).max(1);

    if max_scroll <= 0 || total_count == 0 {
        return ScrollbarMetrics {
            track_left,
            track_top,
            track_right,
            track_bottom,
            thumb_top: track_top,
            thumb_bottom: track_top,
            thumb_h: 0,
            max_scroll: 0,
            page_items,
            needed: false,
        };
    }

    let min_thumb_h = 24;
    let thumb_h = ((page_items as f32 / total_count as f32) * track_height as f32) as i32;
    let thumb_h = thumb_h.clamp(min_thumb_h, track_height);

    let scroll_range = track_height - thumb_h;
    let cur_scroll = tab.scroll_pos.clamp(0, max_scroll);
    let thumb_top =
        track_top + ((cur_scroll as f32 / max_scroll as f32) * scroll_range as f32) as i32;
    let thumb_bottom = thumb_top + thumb_h;

    ScrollbarMetrics {
        track_left,
        track_top,
        track_right,
        track_bottom,
        thumb_top,
        thumb_bottom,
        thumb_h,
        max_scroll,
        page_items,
        needed: true,
    }
}

fn calc_scrollbar_metrics(width: i32, height: i32, tab: &Tab) -> ScrollbarMetrics {
    let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;
    let pane_rect = RECT {
        left: main_left,
        top: CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT,
        right: width - CARD_MARGIN,
        bottom: height - CARD_MARGIN - STATUS_HEIGHT,
    };
    calc_pane_scrollbar_metrics(pane_rect, tab)
}
unsafe fn update_scroll_info(hwnd: HWND, state: &mut State) {
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
        let metrics = calc_scrollbar_metrics(rect.right, rect.bottom, tab);
        tab.scroll_pos = tab.scroll_pos.clamp(0, metrics.max_scroll);
    }
}
unsafe fn open_selected_item(hwnd: HWND, state: &mut State) {
    let tab = match state.tabs.get_mut(state.active_tab) {
        Some(t) => t,
        None => return,
    };

    let items = tab.selected_items();
    if items.is_empty() {
        return;
    }

    if items.len() == 1 && items[0].is_dir {
        let path = items[0].path.clone();
        if tab.nav.go(path) {
            if !tab.search_query.is_empty() {
                tab.search_query.clear();
                set_control_text(hwnd, SEARCH_ID, "");
            }
            tab.reload(state.show_hidden);
            tab.clear_selection();
            tab.scroll_pos = 0;
            set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
            ensure_dir_watcher(hwnd, state);
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        return;
    }

    for item in items {
        if item.is_dir {
            let new_tab = Tab::new(item.path, state.show_hidden, state.settings.view_mode);
            state.tabs.push(new_tab);
        } else if is_exe_file(&item.path) {
            let parent = item.path.parent().unwrap_or(&item.path);
            let _ = std::process::Command::new(&item.path)
                .current_dir(parent)
                .spawn();
        } else if state.settings.open_text_in_editor && is_text_or_code_file(&item.path) {
            open_with_preferred_editor(hwnd, state, &item.path);
        } else {
            let path_clone = item.path.clone();
            std::thread::spawn(move || {
                let path_w = wide(&path_clone.to_string_lossy());
                let dir_w = path_clone.parent().map(|d| wide(&d.to_string_lossy()));
                let open_w = wide("open");
                unsafe {
                    ShellExecuteW(
                        std::ptr::null_mut(),
                        open_w.as_ptr(),
                        path_w.as_ptr(),
                        ptr::null(),
                        dir_w.as_ref().map(|d| d.as_ptr()).unwrap_or(ptr::null()),
                        SW_SHOWNORMAL,
                    );
                }
            });
        }
    }
    InvalidateRect(hwnd, ptr::null(), 0);
}
pub(crate) fn format_size(bytes: u64, is_dir: bool) -> String {
    if is_dir && bytes == 0 {
        return String::new();
    }
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    }
}

/// Timezone offset for Western Indonesia Time (WIB / Jakarta, UTC+7) in seconds (7 * 3600 = 25,200s).
pub(crate) const TIMEZONE_OFFSET_SECS: i64 = 7 * 3600;

fn format_filetime(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let utc_seconds = (ft / 10_000_000).saturating_sub(11_644_473_600) as i64;
    let local_seconds = (utc_seconds + TIMEZONE_OFFSET_SECS).max(0) as u64;
    let days = (local_seconds / 86_400) as i64;
    let time = local_seconds % 86_400;
    let (year, month, day) = unpack_days_to_ymd(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        time / 3600,
        (time % 3600) / 60
    )
}

fn classify_entry(name_w: &[u16], is_dir: bool) -> (&'static str, COLORREF) {
    if is_dir {
        return ("Folder", CLR_ICON_FOLDER);
    }

    let ext = name_w
        .iter()
        .rposition(|&c| c == b'.' as u16)
        .map(|idx| &name_w[idx + 1..]);
    match ext {
        Some(slice) => {
            let mut buf = [0u8; 16];
            let len = slice.len().min(15);
            for i in 0..len {
                let c = slice[i];
                buf[i] = if (b'A' as u16..=b'Z' as u16).contains(&c) {
                    (c + 32) as u8
                } else if c < 128 {
                    c as u8
                } else {
                    0
                };
            }
            match &buf[..len] {
                b"rs" | b"toml" | b"json" | b"js" | b"ts" | b"py" | b"c" | b"cpp" | b"h"
                | b"html" | b"css" | b"md" => ("Code", CLR_ICON_CODE),
                b"txt" | b"doc" | b"docx" | b"pdf" | b"rtf" | b"xls" | b"xlsx" | b"ppt"
                | b"pptx" | b"csv" => ("Document", CLR_ICON_DOC),
                b"png" | b"jpg" | b"jpeg" | b"gif" | b"svg" | b"webp" | b"ico" | b"mp4"
                | b"mkv" | b"avi" | b"mp3" | b"wav" | b"flac" => ("Media", CLR_ICON_MEDIA),
                b"zip" | b"tar" | b"gz" | b"rar" | b"7z" | b"bz2" | b"xz" => {
                    ("Archive", CLR_ICON_ARCHIVE)
                }
                b"exe" | b"msi" | b"bat" | b"cmd" | b"ps1" | b"dll" => {
                    ("Application", CLR_ICON_EXE)
                }
                _ => ("File", CLR_SECONDARY_TEXT),
            }
        }
        None => ("File", CLR_SECONDARY_TEXT),
    }
}

pub(crate) fn compute_file_sha256(path: &Path) -> Option<String> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path).ok()?;
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut buf = [0u8; 64 * 1024];
    let mut total_len: u64 = 0;
    let mut chunk = [0u8; 64];
    let mut chunk_pos = 0;

    let process_block = |h: &mut [u32; 8], block: &[u8]| {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    };

    while let Ok(n) = file.read(&mut buf) {
        if n == 0 {
            break;
        }
        total_len += n as u64;
        let mut offset = 0;
        while offset < n {
            let take = (64 - chunk_pos).min(n - offset);
            chunk[chunk_pos..chunk_pos + take].copy_from_slice(&buf[offset..offset + take]);
            chunk_pos += take;
            offset += take;
            if chunk_pos == 64 {
                process_block(&mut h, &chunk);
                chunk_pos = 0;
            }
        }
    }

    let total_bits = total_len * 8;
    chunk[chunk_pos] = 0x80;
    chunk_pos += 1;
    if chunk_pos > 56 {
        chunk[chunk_pos..64].fill(0);
        process_block(&mut h, &chunk);
        chunk.fill(0);
    } else {
        chunk[chunk_pos..56].fill(0);
    }
    chunk[56..64].copy_from_slice(&total_bits.to_be_bytes());
    process_block(&mut h, &chunk);

    Some(format!(
        "{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}{:08x}",
        h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]
    ))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_child = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_child)?;
        } else {
            std::fs::copy(entry.path(), dest_child)?;
        }
    }
    Ok(())
}

fn duplicate_entry(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let stem = path.file_stem()?.to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let is_dir = path.is_dir();

    let mut counter = 1;
    let mut target = parent.join(format!("{stem} - Copy{ext}"));
    while target.exists() {
        counter += 1;
        target = parent.join(format!("{stem} - Copy ({counter}){ext}"));
    }

    if is_dir {
        copy_dir_recursive(path, &target).ok()?;
    } else {
        std::fs::copy(path, &target).ok()?;
    }
    Some(target)
}

fn create_new_file(dir: &Path, name: &str, ext: &str) -> Option<PathBuf> {
    let mut counter = 1;
    let mut candidate = dir.join(format!("{name}.{ext}"));
    while candidate.exists() {
        counter += 1;
        candidate = dir.join(format!("{name} ({counter}).{ext}"));
    }
    std::fs::write(&candidate, b"").ok()?;
    Some(candidate)
}

fn is_text_or_code_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "toml"
            | "json"
            | "yaml"
            | "yml"
            | "xml"
            | "csv"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "py"
            | "c"
            | "cpp"
            | "h"
            | "hpp"
            | "cs"
            | "go"
            | "html"
            | "css"
            | "scss"
            | "less"
            | "ini"
            | "cfg"
            | "conf"
            | "log"
            | "env"
            | "sh"
            | "bash"
            | "zsh"
            | "bat"
            | "cmd"
            | "ps1"
            | "sql"
            | "lua"
            | "swift"
            | "kt"
            | "dockerfile"
            | "gitignore"
            | "lock"
            | "svg"
            | ""
    )
}

fn open_with_zed(hwnd: HWND, path: &Path) {
    let arg = wide(&format!("\"{}\"", path.to_string_lossy()));
    let zed_full = std::env::var_os("LOCALAPPDATA")
        .map(|l| PathBuf::from(l).join("Programs\\Zed\\bin\\Zed.exe"))
        .filter(|p| p.is_file());

    let exe = if let Some(ref p) = zed_full {
        wide(&p.to_string_lossy())
    } else {
        wide("Zed.exe")
    };

    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            exe.as_ptr(),
            arg.as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn open_with_codium(hwnd: HWND, path: &Path) {
    let arg = wide(&format!("\"{}\"", path.to_string_lossy()));
    let codium_full = PathBuf::from("C:\\Program Files\\VSCodium\\VSCodium.exe");
    let exe = if codium_full.is_file() {
        wide(&codium_full.to_string_lossy())
    } else {
        wide("codium.cmd")
    };

    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            exe.as_ptr(),
            arg.as_ptr(),
            ptr::null(),
            SW_SHOWNORMAL,
        );
    }
}

fn open_with_notepad(hwnd: HWND, path: &Path) {
    let arg = wide(&format!("\"{}\"", path.to_string_lossy()));
    let exe = wide("notepad.exe");
    unsafe {
        ShellExecuteW(
            hwnd,
            wide("open").as_ptr(),
            exe.as_ptr(),
            arg.as_ptr(),
            ptr::null(),
            1,
        );
    }
}

fn show_shortcuts_guide(hwnd: HWND) {
    let title = wide("Rovik Shortcuts");
    let msg = wide(
        "Navigation:\n\
        Enter / Double Click: Open item\n\
        Backspace / Alt+Left: Go Back\n\
        Alt+Right: Go Forward\n\
        Alt+Up: Parent Directory\n\
        Ctrl+L: Focus Path Bar\n\
        Ctrl+F: Focus Filter\n\
        F5 / Ctrl+R: Refresh\n\n\
        Selection & Views:\n\
        Ctrl+A: Select All\n\
        Shift+Click / Shift+Arrows: Range Selection\n\
        Space: Quick Look Preview\n\
        Ctrl+Shift+1: Details View\n\
        Ctrl+Shift+2: Compact View\n\
        Ctrl+Shift+3: Grid View\n\
        F10: Dual Pane Mode\n\n\
        Operations:\n\
        Ctrl+C / Ctrl+X / Ctrl+V: Copy / Cut / Paste\n\
        Delete: Move to Recycle Bin\n\
        Shift+Delete: Permanent Delete\n\
        F2: Rename\n\
        Ctrl+,: Settings\n\
        F1: This Shortcuts Guide",
    );
    unsafe {
        MessageBoxW(
            hwnd,
            msg.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

unsafe fn set_clipboard_files(hwnd: HWND, paths: &[PathBuf], is_cut: bool) -> bool {
    if paths.is_empty() {
        return false;
    }
    let mut wide_chars: Vec<u16> = Vec::new();
    for p in paths {
        for c in p.as_os_str().encode_wide() {
            wide_chars.push(c);
        }
        wide_chars.push(0);
    }
    wide_chars.push(0);

    let dropfiles_size = std::mem::size_of::<DROPFILES>();
    let total_bytes = dropfiles_size + wide_chars.len() * 2;

    let h_mem = GlobalAlloc(GMEM_MOVEABLE | 0x0040, total_bytes);
    if h_mem.is_null() {
        return false;
    }

    let ptr = GlobalLock(h_mem) as *mut u8;
    if ptr.is_null() {
        GlobalFree(h_mem);
        return false;
    }

    let df = ptr as *mut DROPFILES;
    (*df).pFiles = dropfiles_size as u32;
    (*df).pt = POINT { x: 0, y: 0 };
    (*df).fNC = 0;
    (*df).fWide = 1;

    let target_slice =
        std::slice::from_raw_parts_mut(ptr.add(dropfiles_size) as *mut u16, wide_chars.len());
    target_slice.copy_from_slice(&wide_chars);

    GlobalUnlock(h_mem);

    let format_drop_effect = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
    let h_effect = if format_drop_effect != 0 {
        let he = GlobalAlloc(GMEM_MOVEABLE | 0x0040, 4);
        if !he.is_null() {
            let p = GlobalLock(he) as *mut u32;
            if !p.is_null() {
                *p = if is_cut {
                    DROPEFFECT_MOVE
                } else {
                    DROPEFFECT_COPY
                };
                GlobalUnlock(he);
            }
            he
        } else {
            std::ptr::null_mut()
        }
    } else {
        std::ptr::null_mut()
    };

    if OpenClipboard(hwnd) != 0 {
        EmptyClipboard();
        SetClipboardData(CF_HDROP, h_mem as _);
        if !h_effect.is_null() && format_drop_effect != 0 {
            SetClipboardData(format_drop_effect, h_effect as _);
        }
        CloseClipboard();
        true
    } else {
        GlobalFree(h_mem);
        if !h_effect.is_null() {
            GlobalFree(h_effect);
        }
        false
    }
}

unsafe fn get_clipboard_files(hwnd: HWND) -> Option<(Vec<PathBuf>, bool)> {
    if IsClipboardFormatAvailable(CF_HDROP) == 0 {
        return None;
    }
    if OpenClipboard(hwnd) == 0 {
        return None;
    }

    let h_drop = GetClipboardData(CF_HDROP);
    if h_drop.is_null() {
        CloseClipboard();
        return None;
    }

    let count = DragQueryFileW(h_drop as _, 0xFFFFFFFF, std::ptr::null_mut(), 0);
    let mut paths = Vec::new();
    let mut buf = vec![0u16; 1024];

    for i in 0..count {
        let len = DragQueryFileW(h_drop as _, i, buf.as_mut_ptr(), buf.len() as u32);
        if len > 0 {
            let path_str = OsString::from_wide(&buf[..len as usize]);
            paths.push(PathBuf::from(path_str));
        }
    }

    let mut is_cut = false;
    let format_drop_effect = RegisterClipboardFormatW(wide("Preferred DropEffect").as_ptr());
    if format_drop_effect != 0 {
        let h_effect = GetClipboardData(format_drop_effect);
        if !h_effect.is_null() {
            let p = GlobalLock(h_effect) as *const u32;
            if !p.is_null() {
                if *p == DROPEFFECT_MOVE {
                    is_cut = true;
                }
                GlobalUnlock(h_effect);
            }
        }
    }

    CloseClipboard();

    if paths.is_empty() {
        None
    } else {
        Some((paths, is_cut))
    }
}

fn move_entry(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_err() {
        if src.is_dir() {
            copy_dir_recursive(src, dst)?;
            let _ = std::fs::remove_dir_all(src);
        } else {
            std::fs::copy(src, dst)?;
            let _ = std::fs::remove_file(src);
        }
    }
    Ok(())
}

unsafe fn switch_dual_pane(hwnd: HWND, state: &mut State, target_pane: usize) {
    if !state.dual_pane || state.dual_active == target_pane {
        return;
    }
    state.dual_active = target_pane;
    state.active_tab = if state.dual_active == 0 {
        state.left_tab
    } else {
        state.right_tab
    };
    update_controls_for_active_tab(hwnd, state);
    update_scroll_info(hwnd, state);
    ensure_dir_watcher(hwnd, state);
    InvalidateRect(hwnd, std::ptr::null(), 0);
}

fn calculate_folder_size_async(hwnd: HWND, state: &mut State, target_path: PathBuf) {
    let folder_name = target_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let cached_metrics = state
        .folder_size_cache
        .lock()
        .ok()
        .and_then(|c| c.get(&target_path).copied());
    if let Some((sz, f_cnt, d_cnt)) = cached_metrics {
        state.set_toast(format!(
            "{}: {} ({} files, {} folders)",
            folder_name,
            format_size(sz, false),
            f_cnt,
            d_cnt
        ));
        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
            if let Ok(cache) = state.folder_size_cache.lock() {
                tab.apply_folder_sizes(&cache);
            }
        }
        unsafe {
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        return;
    }

    state.set_toast(format!("Calculating size: {}...", folder_name));
    unsafe {
        InvalidateRect(hwnd, ptr::null(), 0);
    }

    let hwnd_val = hwnd as usize;
    let cache_clone = state.folder_size_cache.clone();
    std::thread::spawn(move || {
        let (total_bytes, files_count, dirs_count) =
            properties_ui::get_folder_metrics(&target_path);
        if let Ok(mut map) = cache_clone.lock() {
            map.insert(target_path, (total_bytes, files_count, dirs_count));
        }
        unsafe {
            PostMessageW(hwnd_val as HWND, WM_USER_FOLDER_SIZE_READY, 0, 0);
        }
    });
}

unsafe fn handle_command(hwnd: HWND, state: &mut State, cmd: usize) {
    let focus = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetFocus();
    let path_hwnd = GetDlgItem(hwnd, PATH_ID as i32);
    let search_hwnd = GetDlgItem(hwnd, SEARCH_ID as i32);
    let is_in_edit = focus == path_hwnd || focus == search_hwnd;

    match cmd {
        CMD_FOCUS_PATH => {
            state.is_path_editing = true;
            SetFocus(path_hwnd);
            SendMessageW(path_hwnd, EM_SETSEL, 0, -1);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_FOCUS_SEARCH => {
            SetFocus(search_hwnd);
            SendMessageW(search_hwnd, EM_SETSEL, 0, -1);
        }
        CMD_NEXT_TAB => {
            if state.tabs.len() > 1 {
                state.active_tab = (state.active_tab + 1) % state.tabs.len();
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_PREV_TAB => {
            if state.tabs.len() > 1 {
                state.active_tab = if state.active_tab == 0 {
                    state.tabs.len() - 1
                } else {
                    state.active_tab - 1
                };
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TAB_1..=CMD_TAB_5 => {
            let target = cmd - CMD_TAB_1;
            if target < state.tabs.len() {
                state.active_tab = target;
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_OPEN => {
            open_selected_item(hwnd, state);
        }
        CMD_OPEN_PREFERRED => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                open_with_preferred_editor(hwnd, state, &p);
            }
        }
        CMD_OPEN_NEW_TAB => {
            let target_path = if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    if item.is_dir {
                        Some(item.path)
                    } else {
                        None
                    }
                } else {
                    Some(tab.nav.current().clone())
                }
            } else {
                None
            };
            if let Some(path) = target_path {
                let new_tab = Tab::new(path, state.show_hidden, state.settings.view_mode);
                state.tabs.push(new_tab);
                state.active_tab = state.tabs.len() - 1;
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TAB_CLOSE => {
            if state.tabs.len() > 1 {
                state.tabs.remove(state.active_tab);
                if state.active_tab >= state.tabs.len() {
                    state.active_tab = state.tabs.len() - 1;
                }
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TAB_CLOSE_OTHERS => {
            if state.tabs.len() > 1 && state.active_tab < state.tabs.len() {
                let kept = state.tabs.remove(state.active_tab);
                state.tabs.clear();
                state.tabs.push(kept);
                state.active_tab = 0;
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TAB_DUPLICATE => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = tab.nav.current().clone();
                let new_tab = Tab::new(p, state.show_hidden, tab.view_mode);
                let insert_idx = state.active_tab + 1;
                state.tabs.insert(insert_idx, new_tab);
                state.active_tab = insert_idx;
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_COPY => {
            if is_in_edit {
                SendMessageW(focus, WM_COPY, 0, 0);
            } else if let Some(tab) = state.tabs.get(state.active_tab) {
                let paths: Vec<PathBuf> =
                    tab.selected_items().into_iter().map(|it| it.path).collect();
                if !paths.is_empty() {
                    state.clipboard = Some(ClipboardPayload {
                        paths: paths.clone(),
                        is_cut: false,
                    });
                    set_clipboard_files(hwnd, &paths, false);
                }
            }
        }
        CMD_CUT => {
            if is_in_edit {
                SendMessageW(focus, WM_CUT, 0, 0);
            } else if let Some(tab) = state.tabs.get(state.active_tab) {
                let paths: Vec<PathBuf> =
                    tab.selected_items().into_iter().map(|it| it.path).collect();
                if !paths.is_empty() {
                    state.clipboard = Some(ClipboardPayload {
                        paths: paths.clone(),
                        is_cut: true,
                    });
                    set_clipboard_files(hwnd, &paths, true);
                }
            }
        }
        CMD_PASTE => {
            if is_in_edit {
                SendMessageW(focus, WM_PASTE, 0, 0);
            } else {
                let clip_data = get_clipboard_files(hwnd).or_else(|| {
                    state
                        .clipboard
                        .as_ref()
                        .map(|cp| (cp.paths.clone(), cp.is_cut))
                });
                if let Some((paths, is_cut)) = clip_data {
                    let dest_dir = state
                        .tabs
                        .get(state.active_tab)
                        .map(|t| t.nav.current().clone());
                    if let Some(dest_dir) = dest_dir {
                        let mut move_pairs = Vec::new();
                        let mut copy_dests = Vec::new();
                        for src_path in &paths {
                            if let Some(file_name) = src_path.file_name() {
                                let dest = dest_dir.join(file_name);
                                if is_cut {
                                    if move_entry(src_path, &dest).is_ok() {
                                        move_pairs.push((src_path.clone(), dest));
                                    }
                                } else if src_path.is_dir() {
                                    if copy_dir_recursive(src_path, &dest).is_ok() {
                                        copy_dests.push(dest);
                                    }
                                } else {
                                    if std::fs::copy(src_path, &dest).is_ok() {
                                        copy_dests.push(dest);
                                    }
                                }
                            }
                        }
                        if is_cut {
                            let count = move_pairs.len();
                            state.undo_stack.push(FileOp::Move { pairs: move_pairs });
                            state.redo_stack.clear();
                            state.set_toast(format!("Pasted & moved {} item(s)", count));
                            state.clipboard = None;
                            if OpenClipboard(hwnd) != 0 {
                                EmptyClipboard();
                                CloseClipboard();
                            }
                        } else {
                            let count = copy_dests.len();
                            state.undo_stack.push(FileOp::Copy {
                                dest_paths: copy_dests,
                            });
                            state.redo_stack.clear();
                            state.set_toast(format!("Pasted {} item(s)", count));
                        }
                        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                            tab.reload(state.show_hidden);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
            }
        }
        CMD_DUPLICATE => {
            let items_to_dup: Vec<PathBuf> = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.selected_items().into_iter().map(|it| it.path).collect())
                .unwrap_or_default();
            let mut last_dup = None;
            let mut dup_paths = Vec::new();
            for p in items_to_dup {
                if let Some(new_p) = duplicate_entry(&p) {
                    dup_paths.push(new_p.clone());
                    last_dup = Some(new_p);
                }
            }
            if let Some(new_p) = last_dup {
                let count = dup_paths.len();
                state.undo_stack.push(FileOp::Copy {
                    dest_paths: dup_paths,
                });
                state.redo_stack.clear();
                state.set_toast(format!("Duplicated {} item(s)", count));
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    tab.reload(state.show_hidden);
                    tab.select_path(&new_p);
                }
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_DELETE => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let items = tab.selected_items();
                if !items.is_empty() {
                    if state.settings.confirm_delete {
                        let prompt_msg = if items.len() == 1 {
                            format!("Are you sure you want to delete \"{}\"?", items[0].name)
                        } else {
                            format!(
                                "Are you sure you want to delete these {} items?",
                                items.len()
                            )
                        };
                        let res = MessageBoxW(
                            hwnd,
                            wide(&prompt_msg).as_ptr(),
                            wide("Confirm Delete").as_ptr(),
                            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                        );
                        if res != IDYES {
                            return;
                        }
                    }
                    for item in items {
                        delete_entry(&item.path, item.is_dir, state.settings.recycle_bin);
                    }
                    tab.reload(state.show_hidden);
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_DELETE_PERMANENT => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let items = tab.selected_items();
                if !items.is_empty() {
                    let prompt_msg = if items.len() == 1 {
                        format!("Are you sure you want to permanently delete \"{}\"?\n\nThis cannot be undone.", items[0].name)
                    } else {
                        format!("Are you sure you want to permanently delete these {} items?\n\nThis cannot be undone.", items.len())
                    };
                    let res = MessageBoxW(
                        hwnd,
                        wide(&prompt_msg).as_ptr(),
                        wide("Permanently Delete Item").as_ptr(),
                        MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
                    );
                    if res == IDYES {
                        for item in items {
                            delete_entry(&item.path, item.is_dir, false);
                        }
                        tab.reload(state.show_hidden);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
            }
        }
        CMD_RENAME | CMD_BATCH_RENAME => {
            let sel_items = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.selected_items())
                .unwrap_or_default();
            if sel_items.len() > 1 {
                let rename_items: Vec<batch_rename_ui::BatchRenameItem> = sel_items
                    .into_iter()
                    .map(|it| batch_rename_ui::BatchRenameItem {
                        original_path: it.path.clone(),
                        original_name: it.name.clone(),
                        is_dir: it.is_dir,
                        new_name: it.name,
                    })
                    .collect();
                if let Some(pairs) = batch_rename_ui::show_batch_rename_dialog(hwnd, &rename_items)
                {
                    let mut moved_pairs = Vec::new();
                    let mut errors = 0;
                    for (old_p, new_p) in pairs {
                        if old_p != new_p {
                            if let Err(_) = std::fs::rename(&old_p, &new_p) {
                                errors += 1;
                            } else {
                                moved_pairs.push((old_p, new_p));
                            }
                        }
                    }
                    if !moved_pairs.is_empty() {
                        let count = moved_pairs.len();
                        state.undo_stack.push(FileOp::Move { pairs: moved_pairs });
                        state.redo_stack.clear();
                        if errors > 0 {
                            state.set_toast(format!("Renamed {} items ({} failed)", count, errors));
                        } else {
                            state.set_toast(format!("Successfully renamed {} items", count));
                        }
                        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                            tab.reload(state.show_hidden);
                            if let Ok(cache) = state.folder_size_cache.lock() {
                                tab.apply_folder_sizes(&cache);
                            }
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
            } else {
                let rename_target = state.tabs.get(state.active_tab).and_then(|t| {
                    t.selected_item().map(|it| {
                        let parent = it
                            .path
                            .parent()
                            .unwrap_or_else(|| t.nav.current().as_path())
                            .to_path_buf();
                        (it.path, it.name, it.is_dir, parent)
                    })
                });
                if let Some((orig_path, item_name, is_dir, parent)) = rename_target {
                    if let Some(new_name) = prompt_rename(
                        hwnd,
                        &parent,
                        &item_name,
                        is_dir,
                        state.settings.rename_select_stem,
                        state.settings.warn_ext_change,
                    ) {
                        let new_path = parent.join(&new_name);
                        match rename_path(&orig_path, &new_path) {
                            Ok(()) => {
                                state.undo_stack.push(FileOp::Rename {
                                    original: orig_path,
                                    new_path: new_path.clone(),
                                });
                                state.redo_stack.clear();
                                state.set_toast(format!("Renamed to '{}'", new_name));
                                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                                    tab.reload(state.show_hidden);
                                    if let Ok(cache) = state.folder_size_cache.lock() {
                                        tab.apply_folder_sizes(&cache);
                                    }
                                    tab.select_path(&new_path);
                                }
                                update_scroll_info(hwnd, state);
                                InvalidateRect(hwnd, ptr::null(), 0);
                            }
                            Err(err) => {
                                MessageBoxW(
                                    hwnd,
                                    wide(&err).as_ptr(),
                                    wide("Rename Error").as_ptr(),
                                    MB_OK | MB_ICONERROR,
                                );
                            }
                        }
                    }
                }
            }
        }
        CMD_CALCULATE_FOLDER_SIZE => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    if item.is_dir {
                        calculate_folder_size_async(hwnd, state, item.path.clone());
                    }
                }
            }
        }
        CMD_SETTINGS => {
            if let Some(new_settings) = settings_ui::prompt_settings(hwnd, &state.settings) {
                let _ = new_settings.save();
                let prev_show_hidden = state.settings.show_hidden;
                let prev_folders_first = state.settings.folders_first;
                state.settings = new_settings;
                state.show_hidden = state.settings.show_hidden;
                FOLDERS_FIRST.store(state.settings.folders_first, Ordering::Relaxed);
                if prev_show_hidden != state.show_hidden
                    || prev_folders_first != state.settings.folders_first
                {
                    for tab in &mut state.tabs {
                        tab.reload(state.show_hidden);
                    }
                }
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_NEW_FOLDER => {
            let cur_dir = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.nav.current().clone());
            if let Some(cur_dir) = cur_dir {
                if let Some(new_dir) = create_new_folder(&cur_dir) {
                    let folder_name = new_dir
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    state.undo_stack.push(FileOp::CreateFolder {
                        path: new_dir.clone(),
                    });
                    state.redo_stack.clear();
                    state.set_toast(format!("Created folder '{}'", folder_name));
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        tab.select_path(&new_dir);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_NEW_TEXT_FILE => {
            let cur_dir = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.nav.current().clone());
            if let Some(cur_dir) = cur_dir {
                if let Some(new_file) = create_new_file(&cur_dir, "New Text Document", "txt") {
                    let fname = new_file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    state.undo_stack.push(FileOp::CreateFile {
                        path: new_file.clone(),
                    });
                    state.redo_stack.clear();
                    state.set_toast(format!("Created file '{}'", fname));
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        tab.select_path(&new_file);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_NEW_MD_FILE => {
            let cur_dir = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.nav.current().clone());
            if let Some(cur_dir) = cur_dir {
                if let Some(new_file) = create_new_file(&cur_dir, "New Document", "md") {
                    let fname = new_file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    state.undo_stack.push(FileOp::CreateFile {
                        path: new_file.clone(),
                    });
                    state.redo_stack.clear();
                    state.set_toast(format!("Created file '{}'", fname));
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        tab.select_path(&new_file);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_TERMINAL => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let dir = if let Some(item) = tab.selected_item() {
                    if item.is_dir {
                        item.path
                    } else {
                        tab.nav.current().clone()
                    }
                } else {
                    tab.nav.current().clone()
                };
                open_terminal_in_dir(hwnd, state, &dir);
            }
        }
        CMD_SHOW_IN_EXPLORER => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                show_in_explorer(hwnd, &p);
            }
        }
        CMD_OPEN_ZED => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                open_with_zed(hwnd, &p);
            }
        }
        CMD_OPEN_CODIUM | CMD_OPEN_VSCODE => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                open_with_codium(hwnd, &p);
            }
        }
        CMD_OPEN_NOTEPAD => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    open_with_notepad(hwnd, &item.path);
                }
            }
        }
        CMD_COPY_PATH => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                copy_to_clipboard(hwnd, &p.to_string_lossy());
            }
        }
        CMD_COPY_NAME => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    copy_to_clipboard(hwnd, &item.name);
                }
            }
        }
        CMD_COPY_RELATIVE_PATH => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    copy_to_clipboard(hwnd, &item.name);
                }
            }
        }
        CMD_CHECKSUM_SHA256 => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    if let Some(hash) = compute_file_sha256(&item.path) {
                        copy_to_clipboard(hwnd, &hash);
                        let msg = format!(
                            "File:\n{}\n\nSHA-256 Checksum:\n{}\n\n(Copied to clipboard)",
                            item.name, hash
                        );
                        MessageBoxW(
                            hwnd,
                            wide(&msg).as_ptr(),
                            wide("Rovik — File Checksum").as_ptr(),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                }
            }
        }
        CMD_REFRESH => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.reload(state.show_hidden);
                ensure_dir_watcher(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TOGGLE_HIDDEN => {
            state.show_hidden = !state.show_hidden;
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.reload(state.show_hidden);
            }
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_PROPERTIES => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                show_properties(hwnd, &p);
            }
        }
        CMD_SORT_NAME => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_column = SortColumn::Name;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_TYPE => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_column = SortColumn::Type;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_SIZE => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_column = SortColumn::Size;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_DATE => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_column = SortColumn::Date;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_ASC => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_ascending = true;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_DESC => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.sort_ascending = false;
                tab.sort_current();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_TOGGLE_GROUP_DATE => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.group_by_date = !tab.group_by_date;
                tab.sort_current();
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_HELP_SHORTCUTS => {
            show_shortcuts_guide(hwnd);
        }
        CMD_SELECT_ALL => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.select_all();
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_SORT_DROPDOWN => {
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            let r_right = rect.right - CARD_MARGIN - 12;
            let view_btn_w = 76;
            let sort_btn_w = 76;
            let view_btn_x = r_right - view_btn_w;
            let sort_btn_x = view_btn_x - 8 - sort_btn_w;
            let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
            let btn_h = 30;

            let mut pt = POINT {
                x: sort_btn_x,
                y: nav_y + btn_h,
            };
            ClientToScreen(hwnd, &mut pt);

            let hmenu = CreatePopupMenu();
            let (cur_col, cur_asc) = state
                .tabs
                .get(state.active_tab)
                .map(|t| (t.sort_column, t.sort_ascending))
                .unwrap_or((SortColumn::Name, true));

            let chk = |c: bool| if c { MF_CHECKED } else { MF_UNCHECKED };

            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_col == SortColumn::Name),
                CMD_SORT_NAME,
                wide("Name").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_col == SortColumn::Type),
                CMD_SORT_TYPE,
                wide("Type").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_col == SortColumn::Size),
                CMD_SORT_SIZE,
                wide("Size").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_col == SortColumn::Date),
                CMD_SORT_DATE,
                wide("Date modified").as_ptr(),
            );
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_asc),
                CMD_SORT_ASC,
                wide("Ascending (A-Z / 1-9)").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(!cur_asc),
                CMD_SORT_DESC,
                wide("Descending (Z-A / 9-1)").as_ptr(),
            );
            let grp_chk = state
                .tabs
                .get(state.active_tab)
                .map_or(false, |t| t.group_by_date);
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                hmenu,
                MF_STRING | chk(grp_chk),
                CMD_TOGGLE_GROUP_DATE,
                wide("Group by Date").as_ptr(),
            );

            let sel_cmd = TrackPopupMenu(
                hmenu,
                TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
                pt.x,
                pt.y,
                0,
                hwnd,
                ptr::null(),
            ) as usize;
            DestroyMenu(hmenu);

            if sel_cmd != 0 {
                handle_command(hwnd, state, sel_cmd);
            }
        }
        CMD_VIEW_DROPDOWN => {
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);
            let r_right = rect.right - CARD_MARGIN - 12;
            let view_btn_w = 76;
            let view_btn_x = r_right - view_btn_w;
            let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
            let btn_h = 30;

            let mut pt = POINT {
                x: view_btn_x,
                y: nav_y + btn_h,
            };
            ClientToScreen(hwnd, &mut pt);

            let hmenu = CreatePopupMenu();
            let cur_mode = state
                .tabs
                .get(state.active_tab)
                .map(|t| t.view_mode)
                .unwrap_or(state.settings.view_mode);
            let chk = |c: bool| if c { MF_CHECKED } else { MF_UNCHECKED };

            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_mode == ViewMode::Details),
                CMD_VIEW_DETAILS,
                wide("Details List\tCtrl+Shift+1").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_mode == ViewMode::Compact),
                CMD_VIEW_COMPACT,
                wide("Compact List\tCtrl+Shift+2").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(cur_mode == ViewMode::Grid),
                CMD_VIEW_GRID,
                wide("Grid / Icons\tCtrl+Shift+3").as_ptr(),
            );
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                hmenu,
                MF_STRING | chk(state.dual_pane),
                CMD_TOGGLE_DUAL_PANE,
                wide("Dual-Pane Split View\tF10").as_ptr(),
            );
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                hmenu,
                MF_STRING | chk(state.show_hidden),
                CMD_TOGGLE_HIDDEN,
                wide("Show Hidden Files\tCtrl+H").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(state.settings.show_extensions),
                CMD_TOGGLE_EXTENSIONS,
                wide("Show File Extensions").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING | chk(state.settings.folders_first),
                CMD_TOGGLE_FOLDERS_FIRST,
                wide("Folders on Top").as_ptr(),
            );
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(hmenu, MF_STRING, CMD_REFRESH, wide("Refresh\tF5").as_ptr());

            let sel_cmd = TrackPopupMenu(
                hmenu,
                TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN,
                pt.x,
                pt.y,
                0,
                hwnd,
                ptr::null(),
            ) as usize;
            DestroyMenu(hmenu);

            if sel_cmd != 0 {
                handle_command(hwnd, state, sel_cmd);
            }
        }
        CMD_VIEW_DETAILS => {
            state.settings.view_mode = ViewMode::Details;
            let _ = state.settings.save();
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.view_mode = ViewMode::Details;
            }
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_VIEW_COMPACT => {
            state.settings.view_mode = ViewMode::Compact;
            let _ = state.settings.save();
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.view_mode = ViewMode::Compact;
            }
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_VIEW_GRID => {
            state.settings.view_mode = ViewMode::Grid;
            let _ = state.settings.save();
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                tab.view_mode = ViewMode::Grid;
            }
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_TOGGLE_EXTENSIONS => {
            state.settings.show_extensions = !state.settings.show_extensions;
            let _ = state.settings.save();
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_TOGGLE_FOLDERS_FIRST => {
            state.settings.folders_first = !state.settings.folders_first;
            let _ = state.settings.save();
            FOLDERS_FIRST.store(state.settings.folders_first, Ordering::Relaxed);
            for tab in &mut state.tabs {
                tab.sort_current();
            }
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_PIN_TO_QUICK_ACCESS => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                if let Some(item) = tab.selected_item() {
                    if item.is_dir {
                        let path_str = item.path.to_string_lossy().to_string();
                        if !state.settings.pinned_folders.contains(&path_str) {
                            state.settings.pinned_folders.push(path_str);
                            let _ = state.settings.save();
                            state.sidebar_entries =
                                build_sidebar_entries(&state.settings.pinned_folders);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
                }
            }
        }
        CMD_UNPIN_FROM_QUICK_ACCESS => {
            if let Some(idx) = state.sidebar_hovered {
                if let Some(entry) = state.sidebar_entries.get(idx) {
                    let path_str = entry.path.to_string_lossy().to_string();
                    state.settings.pinned_folders.retain(|p| p != &path_str);
                    let _ = state.settings.save();
                    state.sidebar_entries = build_sidebar_entries(&state.settings.pinned_folders);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_ZIP_COMPRESS => {
            let sel_info = state
                .tabs
                .get(state.active_tab)
                .map(|t| (t.selected_items(), t.nav.current().clone()));
            if let Some((items, parent)) = sel_info {
                if !items.is_empty() {
                    let use_7z = crate::archive::find_7z_binary().is_some();
                    let ext = "zip";
                    let zip_name = if items.len() == 1 {
                        let stem = Path::new(&items[0].name)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy();
                        format!("{stem}.{ext}")
                    } else {
                        let base_name = parent.file_name().unwrap_or_default().to_string_lossy();
                        format!("{base_name}_archive.{ext}")
                    };
                    let dest_zip = parent.join(&zip_name);
                    let paths: Vec<PathBuf> = items.into_iter().map(|it| it.path).collect();

                    match crate::archive::compress(&paths, &dest_zip) {
                        Ok(()) => {
                            let msg = if use_7z {
                                format!("📦 Compressed with 7-Zip: '{}'", zip_name)
                            } else {
                                format!("📦 Compressed to ZIP: '{}'", zip_name)
                            };
                            state.set_toast(msg);
                            state.undo_stack.push(FileOp::CreateFile {
                                path: dest_zip.clone(),
                            });
                            state.redo_stack.clear();
                            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                                tab.reload(state.show_hidden);
                                tab.select_path(&dest_zip);
                            }
                            update_scroll_info(hwnd, state);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                        Err(e) => {
                            MessageBoxW(
                                hwnd,
                                wide(&e).as_ptr(),
                                wide("Archive Error").as_ptr(),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                    }
                }
            }
        }
        CMD_ZIP_EXTRACT_HERE => {
            let sel_info = state
                .tabs
                .get(state.active_tab)
                .map(|t| (t.selected_items(), t.nav.current().clone()));
            if let Some((items, parent)) = sel_info {
                let mut err = None;
                let mut count = 0;
                for item in &items {
                    if crate::archive::is_archive_file(&item.path) {
                        if let Err(e) = crate::archive::extract(&item.path, &parent) {
                            err = Some(e);
                            break;
                        }
                        count += 1;
                    }
                }
                if let Some(e) = err {
                    MessageBoxW(
                        hwnd,
                        wide(&e).as_ptr(),
                        wide("Extract Error").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                } else if count > 0 {
                    state.set_toast(format!("📦 Extracted {} archive(s)", count));
                }
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    tab.reload(state.show_hidden);
                }
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_ZIP_EXTRACT_FOLDER => {
            let sel_info = state
                .tabs
                .get(state.active_tab)
                .map(|t| (t.selected_items(), t.nav.current().clone()));
            if let Some((items, parent)) = sel_info {
                let mut err = None;
                let mut count = 0;
                for item in &items {
                    if crate::archive::is_archive_file(&item.path) {
                        let stem = Path::new(&item.name)
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy();
                        let dest = parent.join(&*stem);
                        if let Err(e) = crate::archive::extract(&item.path, &dest) {
                            err = Some(e);
                            break;
                        }
                        count += 1;
                    }
                }
                if let Some(e) = err {
                    MessageBoxW(
                        hwnd,
                        wide(&e).as_ptr(),
                        wide("Extract Error").as_ptr(),
                        MB_OK | MB_ICONERROR,
                    );
                } else if count > 0 {
                    state.set_toast(format!("📦 Extracted {} archive(s) to folder", count));
                }
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    tab.reload(state.show_hidden);
                }
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_CREATE_SHORTCUT => {
            let sel_info = state
                .tabs
                .get(state.active_tab)
                .map(|t| (t.selected_items(), t.nav.current().clone()));
            if let Some((items, parent)) = sel_info {
                let mut last_link = None;
                for item in items {
                    let stem = item
                        .path
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| item.name.clone());
                    let mut shortcut_name = format!("{stem} - Shortcut.lnk");
                    let mut link_path = parent.join(&shortcut_name);
                    let mut counter = 2;
                    while link_path.exists() {
                        shortcut_name = format!("{stem} - Shortcut ({counter}).lnk");
                        link_path = parent.join(&shortcut_name);
                        counter += 1;
                    }
                    match crate::fs::shell_ext::create_shortcut(&item.path, &link_path) {
                        Ok(()) => {
                            state.undo_stack.push(FileOp::CreateShortcut {
                                path: link_path.clone(),
                            });
                            state.redo_stack.clear();
                            last_link = Some(link_path);
                        }
                        Err(e) => {
                            MessageBoxW(
                                hwnd,
                                wide(&e).as_ptr(),
                                wide("Create Shortcut Error").as_ptr(),
                                MB_OK | MB_ICONERROR,
                            );
                            break;
                        }
                    }
                }
                if let Some(ref link) = last_link {
                    state.set_toast(format!(
                        "🔗 Created shortcut '{}'",
                        link.file_name().unwrap_or_default().to_string_lossy()
                    ));
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        tab.select_path(link);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_OPEN_AS => {
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let p = if let Some(item) = tab.selected_item() {
                    item.path
                } else {
                    tab.nav.current().clone()
                };
                let wide_path = wide(&p.to_string_lossy());
                let wide_openas = wide("openas");
                ShellExecuteW(
                    hwnd,
                    wide_openas.as_ptr(),
                    wide_path.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    SW_SHOWNORMAL,
                );
            }
        }
        CMD_UNDO => {
            if is_in_edit {
                SendMessageW(focus, WM_UNDO, 0, 0);
            } else if let Some(op) = state.undo_stack.pop() {
                let mut redo_op = None;
                let mut toast_msg = String::new();
                match &op {
                    FileOp::Copy { dest_paths } => {
                        for p in dest_paths {
                            if p.is_dir() {
                                let _ = std::fs::remove_dir_all(p);
                            } else {
                                let _ = std::fs::remove_file(p);
                            }
                        }
                        toast_msg = format!("⟲ Undo: Removed {} copied item(s)", dest_paths.len());
                        redo_op = Some(op.clone());
                    }
                    FileOp::Move { pairs } => {
                        for (src, dest) in pairs {
                            if dest.exists() {
                                let _ = move_entry(dest, src);
                            }
                        }
                        toast_msg = format!("⟲ Undo: Restored {} moved item(s)", pairs.len());
                        redo_op = Some(op.clone());
                    }
                    FileOp::Rename { original, new_path } => {
                        if new_path.exists() {
                            let _ = rename_path(new_path, original);
                            toast_msg = format!(
                                "⟲ Undo: Renamed back to '{}'",
                                original.file_name().unwrap_or_default().to_string_lossy()
                            );
                            redo_op = Some(op.clone());
                        }
                    }
                    FileOp::CreateFolder { path } => {
                        if path.is_dir() {
                            let _ = std::fs::remove_dir_all(path);
                            toast_msg = format!(
                                "⟲ Undo: Removed folder '{}'",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            redo_op = Some(op.clone());
                        }
                    }
                    FileOp::CreateFile { path } | FileOp::CreateShortcut { path } => {
                        if path.exists() {
                            let _ = std::fs::remove_file(path);
                            toast_msg = format!(
                                "⟲ Undo: Removed '{}'",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            redo_op = Some(op.clone());
                        }
                    }
                }
                if let Some(ro) = redo_op {
                    state.redo_stack.push(ro);
                }
                state.set_toast(toast_msg);
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    tab.reload(state.show_hidden);
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            } else {
                state.set_toast("Nothing to undo".to_string());
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        }
        CMD_REDO => {
            if !is_in_edit {
                if let Some(op) = state.redo_stack.pop() {
                    let mut undo_op = None;
                    let mut toast_msg = String::new();
                    match &op {
                        FileOp::Copy { dest_paths: _ } => {
                            toast_msg = "↷ Redo: Completed".to_string();
                            undo_op = Some(op.clone());
                        }
                        FileOp::Move { pairs } => {
                            for (src, dest) in pairs {
                                if src.exists() {
                                    let _ = move_entry(src, dest);
                                }
                            }
                            toast_msg = format!("↷ Redo: Moved {} item(s)", pairs.len());
                            undo_op = Some(op.clone());
                        }
                        FileOp::Rename { original, new_path } => {
                            if original.exists() {
                                let _ = rename_path(original, new_path);
                                toast_msg = format!(
                                    "↷ Redo: Renamed to '{}'",
                                    new_path.file_name().unwrap_or_default().to_string_lossy()
                                );
                                undo_op = Some(op.clone());
                            }
                        }
                        FileOp::CreateFolder { path } => {
                            let _ = std::fs::create_dir_all(path);
                            toast_msg = format!(
                                "↷ Redo: Created folder '{}'",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            undo_op = Some(op.clone());
                        }
                        FileOp::CreateFile { path } => {
                            let _ = std::fs::File::create(path);
                            toast_msg = format!(
                                "↷ Redo: Created file '{}'",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            );
                            undo_op = Some(op.clone());
                        }
                        FileOp::CreateShortcut { path: _ } => {
                            toast_msg = "↷ Redo: Shortcut".to_string();
                            undo_op = Some(op.clone());
                        }
                    }
                    if let Some(uo) = undo_op {
                        state.undo_stack.push(uo);
                    }
                    state.set_toast(toast_msg);
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                } else {
                    state.set_toast("Nothing to redo".to_string());
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        400..=449 => {
            let idx = cmd - CMD_OPEN_WITH_BASE;
            if let Some(opt) = state.cached_open_with.get(idx).cloned() {
                if opt.is_choose_another {
                    handle_command(hwnd, state, CMD_OPEN_AS);
                } else if opt.is_default || opt.exe_path.is_none() {
                    open_selected_item(hwnd, state);
                } else if let Some(exe) = opt.exe_path {
                    if let Some(tab) = state.tabs.get(state.active_tab) {
                        if let Some(sel) = tab.selected_item() {
                            let _ = std::process::Command::new(&exe).arg(&sel.path).spawn();
                        }
                    }
                }
            }
        }
        450..=499 => {
            let idx = cmd - CMD_SEND_TO_BASE;
            if let Some(dst) = state.cached_send_to.get(idx).cloned() {
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    let items = tab.selected_items();
                    if dst.is_desktop_shortcut {
                        if let Some(desktop) = dst.target_dir {
                            let mut count = 0;
                            for it in &items {
                                let stem = it
                                    .path
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_else(|| it.name.clone());
                                let mut shortcut_name = format!("{stem} - Shortcut.lnk");
                                let mut link_path = desktop.join(&shortcut_name);
                                let mut counter = 2;
                                while link_path.exists() {
                                    shortcut_name = format!("{stem} - Shortcut ({counter}).lnk");
                                    link_path = desktop.join(&shortcut_name);
                                    counter += 1;
                                }
                                if crate::fs::shell_ext::create_shortcut(&it.path, &link_path)
                                    .is_ok()
                                {
                                    state
                                        .undo_stack
                                        .push(FileOp::CreateShortcut { path: link_path });
                                    count += 1;
                                }
                            }
                            state.set_toast(format!("🔗 Sent {} shortcut(s) to Desktop", count));
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    } else if let Some(target_dir) = dst.target_dir {
                        let mut dest_paths = Vec::new();
                        for it in &items {
                            let dest = target_dir.join(&it.name);
                            let ok = if it.is_dir {
                                copy_dir_recursive(&it.path, &dest).is_ok()
                            } else {
                                std::fs::copy(&it.path, &dest).is_ok()
                            };
                            if ok {
                                dest_paths.push(dest);
                            }
                        }
                        let c = dest_paths.len();
                        state.undo_stack.push(FileOp::Copy { dest_paths });
                        state.redo_stack.clear();
                        state.set_toast(format!("📁 Sent {} item(s) to {}", c, dst.title));
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
            }
        }
        CMD_TOGGLE_DUAL_PANE => {
            state.dual_pane = !state.dual_pane;
            if state.dual_pane {
                if state.tabs.len() < 2 {
                    let cur_p = state
                        .tabs
                        .get(state.active_tab)
                        .map(|t| t.nav.current().clone())
                        .unwrap_or_else(|| PathBuf::from("C:\\"));
                    let new_tab = Tab::new(cur_p, state.show_hidden, state.settings.view_mode);
                    state.tabs.push(new_tab);
                }
                state.left_tab = 0;
                state.right_tab = 1.min(state.tabs.len() - 1);
                state.dual_active = 0;
                state.active_tab = state.left_tab;
            }
            update_controls_for_active_tab(hwnd, state);
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        _ => {}
    }
}
unsafe fn show_tab_context_menu(hwnd: HWND, state: &mut State, pt: POINT, tab_idx: usize) {
    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    let can_close = state.tabs.len() > 1;
    let close_flag = if can_close {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };

    AppendMenuW(
        hmenu,
        close_flag,
        CMD_TAB_CLOSE,
        wide("Close Tab\tCtrl+W").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        close_flag,
        CMD_TAB_CLOSE_OTHERS,
        wide("Close Other Tabs").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_TAB_DUPLICATE,
        wide("Duplicate Tab").as_ptr(),
    );
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_OPEN_ZED,
        wide("Open Tab in Zed").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_OPEN_CODIUM,
        wide("Open Tab in VSCodium").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_TERMINAL,
        wide("Open in Terminal\tCtrl+Shift+T").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_COPY_PATH,
        wide("Copy Tab Path\tCtrl+Shift+C").as_ptr(),
    );

    let cmd = TrackPopupMenu(
        hmenu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    ) as usize;

    DestroyMenu(hmenu);

    if cmd != 0 {
        if cmd == CMD_TAB_CLOSE_OTHERS {
            if state.tabs.len() > 1 && tab_idx < state.tabs.len() {
                let kept = state.tabs.remove(tab_idx);
                state.tabs.clear();
                state.tabs.push(kept);
                state.active_tab = 0;
                update_controls_for_active_tab(hwnd, state);
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
        } else {
            state.active_tab = tab_idx;
            handle_command(hwnd, state, cmd);
        }
    }
}

unsafe fn show_sidebar_context_menu(hwnd: HWND, state: &mut State, pt: POINT, sb_idx: usize) {
    let entry = match state.sidebar_entries.get(sb_idx) {
        Some(e) => e.clone(),
        None => return,
    };
    state.sidebar_hovered = Some(sb_idx);

    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    AppendMenuW(hmenu, MF_STRING, CMD_OPEN, wide("Open\tEnter").as_ptr());
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_OPEN_NEW_TAB,
        wide("Open in New Tab\tCtrl+T").as_ptr(),
    );
    AppendMenuW(hmenu, MF_STRING, CMD_OPEN_ZED, wide("Open in Zed").as_ptr());
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_OPEN_CODIUM,
        wide("Open in VSCodium").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_TERMINAL,
        wide("Open in Terminal\tCtrl+Shift+T").as_ptr(),
    );
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_SHOW_IN_EXPLORER,
        wide("Show in Windows Explorer").as_ptr(),
    );
    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_COPY_PATH,
        wide("Copy Path\tCtrl+Shift+C").as_ptr(),
    );

    if entry.is_pinned {
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_UNPIN_FROM_QUICK_ACCESS,
            wide("Unpin from Quick Access").as_ptr(),
        );
    }

    AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
    AppendMenuW(
        hmenu,
        MF_STRING,
        CMD_PROPERTIES,
        wide("Properties\tAlt+Enter").as_ptr(),
    );

    let cmd = TrackPopupMenu(
        hmenu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    ) as usize;

    DestroyMenu(hmenu);

    match cmd {
        CMD_OPEN => {
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                if tab.nav.go(entry.path) {
                    tab.search_query.clear();
                    set_control_text(hwnd, SEARCH_ID, "");
                    tab.reload(state.show_hidden);
                    tab.clear_selection();
                    tab.scroll_pos = 0;
                    set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
        }
        CMD_OPEN_NEW_TAB => {
            let new_tab = Tab::new(entry.path, state.show_hidden, state.settings.view_mode);
            state.tabs.push(new_tab);
            state.active_tab = state.tabs.len() - 1;
            update_controls_for_active_tab(hwnd, state);
            update_scroll_info(hwnd, state);
            InvalidateRect(hwnd, ptr::null(), 0);
        }
        CMD_UNPIN_FROM_QUICK_ACCESS => {
            handle_command(hwnd, state, CMD_UNPIN_FROM_QUICK_ACCESS);
        }
        CMD_OPEN_ZED => {
            open_with_zed(hwnd, &entry.path);
        }
        CMD_OPEN_CODIUM => {
            open_with_codium(hwnd, &entry.path);
        }
        CMD_TERMINAL => {
            open_terminal_in_dir(hwnd, state, &entry.path);
        }
        CMD_SHOW_IN_EXPLORER => {
            show_in_explorer(hwnd, &entry.path);
        }
        CMD_COPY_PATH => {
            copy_to_clipboard(hwnd, &entry.path.to_string_lossy());
        }
        CMD_PROPERTIES => {
            show_properties(hwnd, &entry.path);
        }
        _ => {}
    }
}

unsafe fn show_context_menu(hwnd: HWND, state: &mut State, pt: POINT, item_clicked: bool) {
    let hmenu = CreatePopupMenu();
    if hmenu.is_null() {
        return;
    }

    if item_clicked {
        let items = state
            .tabs
            .get(state.active_tab)
            .map(|t| t.selected_items())
            .unwrap_or_default();
        let count = items.len();
        let (is_dir, is_text, is_archive) = if count == 1 {
            let item = &items[0];
            let is_t = !item.is_dir && is_text_or_code_file(&item.path);
            let is_a = !item.is_dir && crate::archive::is_archive_file(&item.path);
            (item.is_dir, is_t, is_a)
        } else {
            let any_archive = items
                .iter()
                .any(|it| !it.is_dir && crate::archive::is_archive_file(&it.path));
            (false, false, any_archive)
        };

        if is_text {
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_OPEN_ZED,
                wide("Open with Zed (Preferred)\tCtrl+Shift+Z").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_OPEN_CODIUM,
                wide("Open with VSCodium").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_OPEN,
                wide("Open (Default App)\tEnter").as_ptr(),
            );
        } else if is_dir {
            AppendMenuW(hmenu, MF_STRING, CMD_OPEN, wide("Open\tEnter").as_ptr());
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_OPEN_NEW_TAB,
                wide("Open in New Tab\tCtrl+T").as_ptr(),
            );
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_PIN_TO_QUICK_ACCESS,
                wide("Pin to Quick Access").as_ptr(),
            );
            AppendMenuW(hmenu, MF_STRING, CMD_OPEN_ZED, wide("Open in Zed").as_ptr());
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_OPEN_CODIUM,
                wide("Open in VSCodium").as_ptr(),
            );
        } else {
            AppendMenuW(hmenu, MF_STRING, CMD_OPEN, wide("Open\tEnter").as_ptr());
        }

        if is_archive {
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_ZIP_EXTRACT_HERE,
                wide("Extract Here\tCtrl+Shift+X").as_ptr(),
            );
            let stem = Path::new(&items[0].name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let label = format!("Extract to {stem}\\");
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_ZIP_EXTRACT_FOLDER,
                wide(&label).as_ptr(),
            );
            AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
        }

        let zip_label = if crate::archive::find_7z_binary().is_some() {
            "Compress with 7-Zip\tCtrl+Shift+U"
        } else {
            "Compress to ZIP\tCtrl+Shift+U"
        };
        AppendMenuW(hmenu, MF_STRING, CMD_ZIP_COMPRESS, wide(zip_label).as_ptr());

        // Dynamic Open With Submenu
        let hopen_menu = CreatePopupMenu();
        if count == 1 && !is_dir {
            state.cached_open_with = crate::fs::shell_ext::get_open_with_handlers(&items[0].path);
            for (idx, opt) in state.cached_open_with.iter().enumerate() {
                let flag = if opt.is_default {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                };
                AppendMenuW(
                    hopen_menu,
                    flag,
                    CMD_OPEN_WITH_BASE + idx,
                    wide(&opt.title).as_ptr(),
                );
            }
        } else {
            AppendMenuW(
                hopen_menu,
                MF_STRING,
                CMD_OPEN_ZED,
                wide("Zed (Code Editor)").as_ptr(),
            );
            AppendMenuW(
                hopen_menu,
                MF_STRING,
                CMD_OPEN_CODIUM,
                wide("VSCodium").as_ptr(),
            );
            AppendMenuW(
                hopen_menu,
                MF_STRING,
                CMD_TERMINAL,
                wide("Windows Terminal").as_ptr(),
            );
        }
        AppendMenuW(
            hmenu,
            MF_POPUP,
            hopen_menu as usize,
            wide("Open with").as_ptr(),
        );

        // Dynamic Send To Submenu
        state.cached_send_to = crate::fs::shell_ext::get_send_to_destinations();
        let hsend_menu = CreatePopupMenu();
        for (idx, dst) in state.cached_send_to.iter().enumerate() {
            AppendMenuW(
                hsend_menu,
                MF_STRING,
                CMD_SEND_TO_BASE + idx,
                wide(&dst.title).as_ptr(),
            );
        }
        AppendMenuW(
            hmenu,
            MF_POPUP,
            hsend_menu as usize,
            wide("Send to").as_ptr(),
        );

        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_TERMINAL,
            wide("Open in Terminal\tCtrl+Shift+T").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_SHOW_IN_EXPLORER,
            wide("Show in Windows Explorer\tCtrl+Shift+E").as_ptr(),
        );
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());

        let cut_label = if count > 1 {
            format!("Cut ({} items)\tCtrl+X", count)
        } else {
            "Cut\tCtrl+X".to_string()
        };
        let copy_label = if count > 1 {
            format!("Copy ({} items)\tCtrl+C", count)
        } else {
            "Copy\tCtrl+C".to_string()
        };
        AppendMenuW(hmenu, MF_STRING, CMD_CUT, wide(&cut_label).as_ptr());
        AppendMenuW(hmenu, MF_STRING, CMD_COPY, wide(&copy_label).as_ptr());

        // Copy & Hash Submenu
        let hcopy_menu = CreatePopupMenu();
        AppendMenuW(
            hcopy_menu,
            MF_STRING,
            CMD_COPY_PATH,
            wide("Copy Full Path\tCtrl+Shift+C").as_ptr(),
        );
        AppendMenuW(
            hcopy_menu,
            MF_STRING,
            CMD_COPY_NAME,
            wide("Copy File Name").as_ptr(),
        );
        AppendMenuW(
            hcopy_menu,
            MF_STRING,
            CMD_COPY_RELATIVE_PATH,
            wide("Copy Relative Path").as_ptr(),
        );
        if !is_dir && count == 1 {
            AppendMenuW(hcopy_menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                hcopy_menu,
                MF_STRING,
                CMD_CHECKSUM_SHA256,
                wide("Compute SHA-256 Checksum\tCtrl+Shift+H").as_ptr(),
            );
        }
        AppendMenuW(
            hmenu,
            MF_POPUP,
            hcopy_menu as usize,
            wide("Copy & Hash").as_ptr(),
        );

        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_CREATE_SHORTCUT,
            wide("Create Shortcut").as_ptr(),
        );
        if count == 1 {
            if is_dir {
                AppendMenuW(
                    hmenu,
                    MF_STRING,
                    CMD_CALCULATE_FOLDER_SIZE,
                    wide("Calculate Folder Size\tSpace").as_ptr(),
                );
            }
            AppendMenuW(hmenu, MF_STRING, CMD_RENAME, wide("Rename\tF2").as_ptr());
            AppendMenuW(
                hmenu,
                MF_STRING,
                CMD_DUPLICATE,
                wide("Duplicate / Clone\tCtrl+D").as_ptr(),
            );
        } else if count > 1 {
            let label = format!("Batch Rename ({} items)...\tF2", count);
            AppendMenuW(hmenu, MF_STRING, CMD_BATCH_RENAME, wide(&label).as_ptr());
        }
        let del_label = if count > 1 {
            format!("Delete ({} items)\tDel", count)
        } else {
            "Delete\tDel".to_string()
        };
        AppendMenuW(hmenu, MF_STRING, CMD_DELETE, wide(&del_label).as_ptr());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_DELETE_PERMANENT,
            wide("Permanently Delete\tShift+Del").as_ptr(),
        );
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_PROPERTIES,
            wide("Properties\tAlt+Enter").as_ptr(),
        );
    } else {
        // Sort Submenu
        let hsort_menu = CreatePopupMenu();
        let (cur_col, cur_asc) = state
            .tabs
            .get(state.active_tab)
            .map(|t| (t.sort_column, t.sort_ascending))
            .unwrap_or((SortColumn::Name, true));

        let chk = |c: bool| if c { MF_CHECKED } else { MF_UNCHECKED };
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(cur_col == SortColumn::Name),
            CMD_SORT_NAME,
            wide("Name").as_ptr(),
        );
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(cur_col == SortColumn::Type),
            CMD_SORT_TYPE,
            wide("Type").as_ptr(),
        );
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(cur_col == SortColumn::Size),
            CMD_SORT_SIZE,
            wide("Size").as_ptr(),
        );
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(cur_col == SortColumn::Date),
            CMD_SORT_DATE,
            wide("Date modified").as_ptr(),
        );
        AppendMenuW(hsort_menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(cur_asc),
            CMD_SORT_ASC,
            wide("Ascending").as_ptr(),
        );
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(!cur_asc),
            CMD_SORT_DESC,
            wide("Descending").as_ptr(),
        );
        let grp_chk = state
            .tabs
            .get(state.active_tab)
            .map_or(false, |t| t.group_by_date);
        AppendMenuW(hsort_menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hsort_menu,
            MF_STRING | chk(grp_chk),
            CMD_TOGGLE_GROUP_DATE,
            wide("Group by Date").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_POPUP,
            hsort_menu as usize,
            wide("Sort by").as_ptr(),
        );

        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_VIEW_DROPDOWN,
            wide("View Layout & Options...").as_ptr(),
        );
        AppendMenuW(hmenu, MF_STRING, CMD_REFRESH, wide("Refresh\tF5").as_ptr());
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());

        // Undo & Redo in Context Menu
        let undo_flag = if state.undo_stack.is_empty() {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        let redo_flag = if state.redo_stack.is_empty() {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        AppendMenuW(hmenu, undo_flag, CMD_UNDO, wide("Undo\tCtrl+Z").as_ptr());
        AppendMenuW(hmenu, redo_flag, CMD_REDO, wide("Redo\tCtrl+Y").as_ptr());
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());

        // New Submenu
        let hnew_menu = CreatePopupMenu();
        AppendMenuW(
            hnew_menu,
            MF_STRING,
            CMD_NEW_FOLDER,
            wide("Folder\tCtrl+Shift+N").as_ptr(),
        );
        AppendMenuW(hnew_menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hnew_menu,
            MF_STRING,
            CMD_NEW_TEXT_FILE,
            wide("Text Document (.txt)").as_ptr(),
        );
        AppendMenuW(
            hnew_menu,
            MF_STRING,
            CMD_NEW_MD_FILE,
            wide("Markdown Document (.md)").as_ptr(),
        );
        AppendMenuW(hmenu, MF_POPUP, hnew_menu as usize, wide("New").as_ptr());

        let can_paste = state.clipboard.is_some() || get_clipboard_files(hwnd).is_some();
        let paste_flag = if can_paste {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        };
        AppendMenuW(hmenu, paste_flag, CMD_PASTE, wide("Paste\tCtrl+V").as_ptr());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_SELECT_ALL,
            wide("Select All\tCtrl+A").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_OPEN_ZED,
            wide("Open in Zed (Preferred)\tCtrl+Shift+Z").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_OPEN_CODIUM,
            wide("Open in VSCodium").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_TERMINAL,
            wide("Open in Terminal\tCtrl+Shift+T").as_ptr(),
        );
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());

        let hidden_chk = if state.show_hidden {
            MF_CHECKED
        } else {
            MF_UNCHECKED
        };
        AppendMenuW(
            hmenu,
            MF_STRING | hidden_chk,
            CMD_TOGGLE_HIDDEN,
            wide("Show Hidden Items\tCtrl+H").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_COPY_PATH,
            wide("Copy Folder Path\tCtrl+Shift+C").as_ptr(),
        );
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_PROPERTIES,
            wide("Properties\tAlt+Enter").as_ptr(),
        );
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_HELP_SHORTCUTS,
            wide("Keyboard Shortcuts Guide\tF1").as_ptr(),
        );
        AppendMenuW(hmenu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(
            hmenu,
            MF_STRING,
            CMD_SETTINGS,
            wide("Settings...\tCtrl+,").as_ptr(),
        );
    }

    let cmd = TrackPopupMenu(
        hmenu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        0,
        hwnd,
        ptr::null(),
    ) as usize;

    DestroyMenu(hmenu);

    if cmd != 0 {
        handle_command(hwnd, state, cmd);
    }
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;

    match msg {
        WM_CREATE => 0,
        WM_SIZE => {
            let width = (lparam as u32 & 0xffff) as i32;
            let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;

            let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
            let input_h = 30;
            let r_right = width - CARD_MARGIN - 12;
            let view_btn_w = 76;
            let sort_btn_w = 76;
            let view_btn_x = r_right - view_btn_w;
            let sort_btn_x = view_btn_x - 8 - sort_btn_w;
            let filter_w = 110;
            let search_x = sort_btn_x - 8 - filter_w;

            let path_x = main_left + 86;
            let path_w = (search_x - 8 - path_x).max(100);

            MoveWindow(
                GetDlgItem(hwnd, PATH_ID as i32),
                path_x + 8,
                nav_y + 4,
                path_w - 16,
                input_h - 8,
                1,
            );
            MoveWindow(
                GetDlgItem(hwnd, SEARCH_ID as i32),
                search_x + 8,
                nav_y + 4,
                filter_w - 16,
                input_h - 8,
                1,
            );

            if !state_ptr.is_null() {
                update_scroll_info(hwnd, &mut *state_ptr);
            }
            InvalidateRect(hwnd, ptr::null(), 0);
            0
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if !state_ptr.is_null() {
                paint(hwnd, &mut *state_ptr);
            }
            0
        }
        WM_USER_NAVIGATE => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    let text = get_control_text(hwnd, PATH_ID);
                    if tab.nav.go_str(&text) {
                        tab.reload(state.show_hidden);
                        ensure_dir_watcher(hwnd, state);
                    } else {
                        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                    }
                    state.is_path_editing = false;
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                SetFocus(hwnd);
            }
            0
        }
        WM_USER_CANCEL_INPUT => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    state.is_path_editing = false;
                    set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                    if !tab.search_query.is_empty() {
                        tab.search_query.clear();
                        set_control_text(hwnd, SEARCH_ID, "");
                        tab.reload(state.show_hidden);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                SetFocus(hwnd);
            }
            0
        }
        WM_COMMAND => {
            if state_ptr.is_null() {
                return 0;
            }
            let id = (wparam & 0xffff) as usize;
            let code = ((wparam >> 16) & 0xffff) as u16;
            let state = &mut *state_ptr;

            if lparam != 0 {
                match id {
                    SEARCH_ID if u32::from(code) == EN_CHANGE => {
                        let query = get_control_text(hwnd, SEARCH_ID).trim().to_string();
                        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                            if query != tab.search_query {
                                tab.search_query = query.clone();
                                if let Some(h) = tab.search_handle.take() {
                                    h.cancel();
                                }
                                tab.search_results.clear();
                                tab.clear_selection();
                                tab.scroll_pos = 0;

                                if !query.is_empty() {
                                    let root = tab.nav.current().clone();
                                    tab.search_handle =
                                        Some(search::search(root, &query, state.show_hidden));
                                }
                                update_scroll_info(hwnd, state);
                                InvalidateRect(hwnd, ptr::null(), 0);
                            }
                        }
                    }
                    PATH_ID if u32::from(code) == EN_SETFOCUS => {
                        state.is_path_editing = true;
                        SendMessageW(GetDlgItem(hwnd, PATH_ID as i32), EM_SETSEL, 0, -1);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                    PATH_ID if u32::from(code) == EN_KILLFOCUS => {
                        state.is_path_editing = false;
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                    _ => {}
                }
            } else {
                handle_command(hwnd, state, id);
            }
            0
        }
        WM_USER_DIR_CHANGED => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    tab.reload(state.show_hidden);
                    if let Ok(cache) = state.folder_size_cache.lock() {
                        tab.apply_folder_sizes(&cache);
                    }
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_USER_FOLDER_SIZE_READY => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                let cache = if let Ok(c) = state.folder_size_cache.lock() {
                    c.clone()
                } else {
                    std::collections::HashMap::new()
                };
                for tab in &mut state.tabs {
                    tab.apply_folder_sizes(&cache);
                }
                if let Some(tab) = state.tabs.get(state.active_tab) {
                    if let Some(item) = tab.selected_item() {
                        if item.is_dir {
                            if let Some(&(sz, f_cnt, d_cnt)) = cache.get(&item.path) {
                                state.set_toast(format!(
                                    "{}: {} ({} files, {} folders)",
                                    item.name,
                                    format_size(sz, false),
                                    f_cnt,
                                    d_cnt
                                ));
                                preview::update_preview_if_open(&item.path);
                            }
                        }
                    }
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_DEVICECHANGE => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                let event = wparam as usize;

                if event == DBT_DEVICEARRIVAL
                    || event == DBT_DEVICEREMOVECOMPLETE
                    || event == DBT_DEVNODES_CHANGED
                {
                    // Automatically detect plugged/unplugged USB drives and refresh sidebar in real time
                    state.sidebar_entries = build_sidebar_entries(&state.settings.pinned_folders);

                    // If the active tab was browsing a drive that was disconnected, safely fall back to Home
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        if !tab.nav.current().exists() {
                            let fallback = std::env::var_os("USERPROFILE")
                                .map(PathBuf::from)
                                .unwrap_or_else(|| PathBuf::from("C:\\"));
                            let _ = tab.nav.go(fallback);
                            set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                        }
                        tab.reload(state.show_hidden);
                        ensure_dir_watcher(hwnd, state);
                        update_scroll_info(hwnd, state);
                    }

                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_TIMER => {
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            let mut need_redraw = false;

            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                if let Some(ref handle) = tab.search_handle {
                    let mut count = 0;
                    while let Ok(res) = handle.receiver.try_recv() {
                        tab.search_results.push(res);
                        count += 1;
                        if count > 32 {
                            break;
                        }
                    }
                    if count > 0 {
                        need_redraw = true;
                    }
                }
            }

            if need_redraw {
                update_scroll_info(hwnd, state);
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_NOTIFY => {
            let nmhdr = lparam as *const NMHDR;
            if !nmhdr.is_null() && (*nmhdr).code == NM_CUSTOMDRAW {
                let nmcd = lparam as *mut NMCUSTOMDRAW;
                match (*nmcd).dwDrawStage {
                    CDDS_PREPAINT => return CDRF_NOTIFYITEMDRAW as LRESULT,
                    CDDS_ITEMPREPAINT => return CDRF_NOTIFYSUBITEMDRAW as LRESULT,
                    stage if (stage & CDDS_SUBITEM) != 0 => {
                        return CDRF_DODEFAULT as LRESULT;
                    }
                    _ => return CDRF_DODEFAULT as LRESULT,
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_KEYDOWN => {
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);

            let tab = match state.tabs.get_mut(state.active_tab) {
                Some(t) => t,
                None => return 0,
            };

            let total_items = tab.items_count();
            let shift_down = (GetKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;
            let ctrl_down = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
            let vk = wparam as u16;

            let header_h = match tab.view_mode {
                ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
                ViewMode::Grid => 0,
            };
            let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + header_h;
            let main_bottom = rect.bottom - CARD_MARGIN - STATUS_HEIGHT;
            let list_height = (main_bottom - main_top).max(0);
            let row_h = match tab.view_mode {
                ViewMode::Details => 34,
                ViewMode::Compact => 26,
                ViewMode::Grid => 100,
            };
            let page_items = (list_height / row_h).max(1) as usize;

            match vk {
                VK_TAB if !ctrl_down && !shift_down => {
                    if state.dual_pane {
                        let next = if state.dual_active == 0 { 1 } else { 0 };
                        switch_dual_pane(hwnd, state, next);
                        return 0;
                    }
                }
                VK_SPACE => {
                    if let Some(item) = tab.selected_item() {
                        if item.is_dir {
                            calculate_folder_size_async(hwnd, state, item.path.clone());
                        }
                        preview::show_or_toggle_preview(hwnd, &item.path);
                    }
                }
                0x41 if ctrl_down => {
                    tab.select_all();
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
                VK_F10 => {
                    handle_command(hwnd, state, CMD_TOGGLE_DUAL_PANE);
                }
                VK_UP => {
                    if total_items > 0 {
                        let cur = tab.focused.unwrap_or(0);
                        let target = cur.saturating_sub(1);
                        if shift_down {
                            tab.select_range(target);
                        } else {
                            tab.select_single(target);
                        }
                        if let Some(disp_idx) = tab.entry_to_display_row(target) {
                            let disp_i = disp_idx as i32;
                            if disp_i < tab.scroll_pos {
                                tab.scroll_pos = disp_i;
                            } else if disp_i >= tab.scroll_pos + page_items as i32 {
                                tab.scroll_pos = (disp_i - page_items as i32 + 1).max(0);
                            }
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_DOWN => {
                    if total_items > 0 {
                        let cur = tab.focused.unwrap_or(0);
                        let target = (cur + 1).min(total_items - 1);
                        if shift_down {
                            tab.select_range(target);
                        } else {
                            tab.select_single(target);
                        }
                        if let Some(disp_idx) = tab.entry_to_display_row(target) {
                            let disp_i = disp_idx as i32;
                            if disp_i < tab.scroll_pos {
                                tab.scroll_pos = disp_i;
                            } else if disp_i >= tab.scroll_pos + page_items as i32 {
                                tab.scroll_pos = (disp_i - page_items as i32 + 1).max(0);
                            }
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_PRIOR => {
                    if total_items > 0 {
                        let cur = tab.focused.unwrap_or(0);
                        let target = cur.saturating_sub(page_items);
                        if shift_down {
                            tab.select_range(target);
                        } else {
                            tab.select_single(target);
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_NEXT => {
                    if total_items > 0 {
                        let cur = tab.focused.unwrap_or(0);
                        let target = (cur + page_items).min(total_items - 1);
                        if shift_down {
                            tab.select_range(target);
                        } else {
                            tab.select_single(target);
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_HOME => {
                    if total_items > 0 {
                        if shift_down {
                            tab.select_range(0);
                        } else {
                            tab.select_single(0);
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_END => {
                    if total_items > 0 {
                        let target = total_items - 1;
                        if shift_down {
                            tab.select_range(target);
                        } else {
                            tab.select_single(target);
                        }
                        if let Some(item) = tab.selected_item() {
                            preview::update_preview_if_open(&item.path);
                        }
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_RETURN => {
                    if GetKeyState(VK_MENU as i32) < 0 {
                        handle_command(hwnd, state, CMD_PROPERTIES);
                    } else {
                        open_selected_item(hwnd, state);
                    }
                }
                VK_DELETE => {
                    handle_command(hwnd, state, CMD_DELETE);
                }
                VK_F2 => {
                    handle_command(hwnd, state, CMD_RENAME);
                }
                VK_F5 => {
                    handle_command(hwnd, state, CMD_REFRESH);
                }
                VK_BACK => {
                    if tab.nav.parent().is_some() {
                        tab.reload(state.show_hidden);
                        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                        ensure_dir_watcher(hwnd, state);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_LEFT if GetKeyState(VK_MENU as i32) < 0 => {
                    if tab.nav.back().is_some() {
                        tab.reload(state.show_hidden);
                        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                        ensure_dir_watcher(hwnd, state);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_RIGHT if GetKeyState(VK_MENU as i32) < 0 => {
                    if tab.nav.forward().is_some() {
                        tab.reload(state.show_hidden);
                        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                        ensure_dir_watcher(hwnd, state);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                VK_ESCAPE => {
                    if preview::is_preview_open() {
                        preview::close_preview();
                    } else if state.is_path_editing {
                        state.is_path_editing = false;
                        set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                    } else if !tab.search_query.is_empty() {
                        tab.search_query.clear();
                        set_control_text(hwnd, SEARCH_ID, "");
                        tab.reload(state.show_hidden);
                        tab.clear_selection();
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
                _ => {}
            }
            0
        }
        WM_MOUSEMOVE => {
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            if !state.tracking_mouse {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                state.tracking_mouse = true;
            }

            let x = (lparam as u32 & 0xffff) as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as i32;
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);

            let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;
            let mut redraw = false;

            // 1. Sidebar hover
            let old_sb = state.sidebar_hovered;
            let (sb_layout, _) = compute_sidebar_layout(&state.sidebar_entries, rect.bottom);
            let mut hovered_sb = None;
            for item in &sb_layout {
                if x >= item.rect.left
                    && x <= item.rect.right
                    && y >= item.rect.top
                    && y <= item.rect.bottom
                {
                    hovered_sb = Some(item.entry_idx);
                    break;
                }
            }
            state.sidebar_hovered = hovered_sb;
            if old_sb != state.sidebar_hovered {
                redraw = true;
            }

            // 1b. Sidebar settings button hover
            let old_sb_btn = state.settings_btn_hovered;
            let s_top = rect.bottom - CARD_MARGIN - 38;
            let s_bottom = rect.bottom - CARD_MARGIN - 8;
            if x >= CARD_MARGIN + 8
                && x < CARD_MARGIN + SIDEBAR_WIDTH - 8
                && y >= s_top
                && y < s_bottom
            {
                state.settings_btn_hovered = true;
            } else {
                state.settings_btn_hovered = false;
            }
            if old_sb_btn != state.settings_btn_hovered {
                redraw = true;
            }

            // 2. Tab Bar hover
            let tab_y = CARD_MARGIN + 4;
            let old_th = state.tab_hovered;
            let old_tch = state.tab_close_hovered;
            let old_tph = state.tab_plus_hovered;

            state.tab_hovered = None;
            state.tab_close_hovered = None;
            state.tab_plus_hovered = false;

            if y >= tab_y && y < tab_y + TAB_BAR_HEIGHT - 4 {
                let mut cur_x = main_left + 10;
                for i in 0..state.tabs.len() {
                    let tab_w = 126;
                    if x >= cur_x && x < cur_x + tab_w {
                        state.tab_hovered = Some(i);
                        if x >= cur_x + tab_w - 24 && x < cur_x + tab_w - 4 {
                            state.tab_close_hovered = Some(i);
                        }
                        break;
                    }
                    cur_x += tab_w + 6;
                }
                if x >= cur_x && x < cur_x + 30 {
                    state.tab_plus_hovered = true;
                }
            }
            if old_th != state.tab_hovered
                || old_tch != state.tab_close_hovered
                || old_tph != state.tab_plus_hovered
            {
                redraw = true;
            }

            // 3. Navigation Buttons hover
            let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
            let btn_h = 30;
            let old_btn = state.btn_hovered;

            let r_right = rect.right - CARD_MARGIN - 12;
            let view_btn_w = 76;
            let sort_btn_w = 76;
            let view_btn_x = r_right - view_btn_w;
            let sort_btn_x = view_btn_x - 8 - sort_btn_w;

            if y >= nav_y && y < nav_y + btn_h {
                if x >= main_left + 12 && x < main_left + 44 {
                    state.btn_hovered = Some(TopButton::Back);
                } else if x >= main_left + 48 && x < main_left + 80 {
                    state.btn_hovered = Some(TopButton::Forward);
                } else if x >= sort_btn_x && x < sort_btn_x + sort_btn_w {
                    state.btn_hovered = Some(TopButton::SortDropdown);
                } else if x >= view_btn_x && x < view_btn_x + view_btn_w {
                    state.btn_hovered = Some(TopButton::ViewDropdown);
                } else {
                    state.btn_hovered = None;
                }
            } else {
                state.btn_hovered = None;
            }
            if old_btn != state.btn_hovered {
                redraw = true;
            }

            // 4. Breadcrumbs hover
            let old_bc = state.hovered_breadcrumb;
            state.hovered_breadcrumb = None;
            if !state.is_path_editing {
                for (idx, crumb) in state.breadcrumbs.iter().enumerate() {
                    if x >= crumb.rect.left
                        && x <= crumb.rect.right
                        && y >= crumb.rect.top
                        && y <= crumb.rect.bottom
                    {
                        state.hovered_breadcrumb = Some(idx);
                        break;
                    }
                }
            }
            if old_bc != state.hovered_breadcrumb {
                redraw = true;
            }

            // 4b. Scrollbar hover & dragging
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let sb_metrics = calc_scrollbar_metrics(rect.right, rect.bottom, tab);
                let old_sb_hov = state.scrollbar_hovered;

                if state.scrollbar_dragging {
                    let travel =
                        (sb_metrics.track_bottom - sb_metrics.track_top - sb_metrics.thumb_h)
                            .max(1);
                    let cur_thumb_y = (y - state.scrollbar_drag_offset_y).clamp(
                        sb_metrics.track_top,
                        sb_metrics.track_bottom - sb_metrics.thumb_h,
                    );
                    let progress = (cur_thumb_y - sb_metrics.track_top) as f32 / travel as f32;
                    let target_pos = (progress * sb_metrics.max_scroll as f32).round() as i32;
                    if target_pos != tab.scroll_pos {
                        tab.scroll_pos = target_pos.clamp(0, sb_metrics.max_scroll);
                        redraw = true;
                    }
                } else {
                    state.scrollbar_hovered = sb_metrics.needed
                        && x >= sb_metrics.track_left - 4
                        && x <= sb_metrics.track_right + 4
                        && y >= sb_metrics.track_top
                        && y <= sb_metrics.track_bottom;
                    if old_sb_hov != state.scrollbar_hovered {
                        redraw = true;
                    }
                }
            }

            // 5. Main rows / grid cards hover
            let old_row = state.hovered_row;
            if let Some(tab) = state.tabs.get(state.active_tab) {
                let header_h = match tab.view_mode {
                    ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
                    ViewMode::Grid => 0,
                };
                let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + header_h;
                let main_bottom = rect.bottom - CARD_MARGIN - STATUS_HEIGHT;

                if x >= main_left
                    && x < rect.right - CARD_MARGIN
                    && y >= main_top
                    && y < main_bottom
                {
                    let total_count = tab.items_count();
                    match tab.view_mode {
                        ViewMode::Details => {
                            let row_idx = ((y - main_top) / 34) as usize + tab.scroll_pos as usize;
                            state.hovered_row = if row_idx < total_count {
                                Some(row_idx)
                            } else {
                                None
                            };
                        }
                        ViewMode::Compact => {
                            let row_idx = ((y - main_top) / 26) as usize + tab.scroll_pos as usize;
                            state.hovered_row = if row_idx < total_count {
                                Some(row_idx)
                            } else {
                                None
                            };
                        }
                        ViewMode::Grid => {
                            let main_w = (rect.right - CARD_MARGIN - main_left - 24).max(120);
                            let cols = (main_w / 116).max(1) as usize;
                            let rel_x = x - (main_left + 12);
                            let rel_y = y - main_top;
                            if rel_x >= 0 && rel_y >= 0 {
                                let col = (rel_x / 116) as usize;
                                let row = (rel_y / 100) as usize + tab.scroll_pos as usize;
                                if col < cols {
                                    let idx = row * cols + col;
                                    state.hovered_row =
                                        if idx < total_count { Some(idx) } else { None };
                                } else {
                                    state.hovered_row = None;
                                }
                            } else {
                                state.hovered_row = None;
                            }
                        }
                    }
                } else {
                    state.hovered_row = None;
                }
            } else {
                state.hovered_row = None;
            }
            // 6. OLE Drag & Drop trigger detection (Mouse & Laptop Touchpad tap-drag)
            if let Some(start_pt) = state.drag_start {
                let lbutton_down = (wparam as usize & 0x0001) != 0
                    || (GetKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0;
                if !lbutton_down {
                    state.drag_start = None;
                    state.drag_candidate = None;
                    state.drag_deferred_select = false;
                    ReleaseCapture();
                } else {
                    let dx = (x - start_pt.x).abs();
                    let dy = (y - start_pt.y).abs();
                    let drag_cx = GetSystemMetrics(SM_CXDRAG).max(4);
                    let drag_cy = GetSystemMetrics(SM_CYDRAG).max(4);
                    if dx >= drag_cx || dy >= drag_cy {
                        state.drag_start = None;
                        state.drag_candidate = None;
                        state.drag_deferred_select = false;
                        ReleaseCapture();

                        let paths: Vec<PathBuf> =
                            if let Some(tab) = state.tabs.get(state.active_tab) {
                                tab.selected_items().into_iter().map(|it| it.path).collect()
                            } else {
                                Vec::new()
                            };

                        if !paths.is_empty() {
                            let (hr, final_effect, shift_at_drop) =
                                drag_drop::start_drag_drop(hwnd, paths.clone());
                            if hr == drag_drop::DRAGDROP_S_DROP {
                                if final_effect == drag_drop::DROPEFFECT_MOVE && shift_at_drop {
                                    for p in &paths {
                                        let is_dir = p.is_dir();
                                        delete_entry(p, is_dir, true);
                                    }
                                }
                                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                                    tab.reload(state.show_hidden);
                                    update_scroll_info(hwnd, state);
                                    InvalidateRect(hwnd, ptr::null(), 0);
                                }
                            }
                        }
                    }
                }
            }

            if old_row != state.hovered_row {
                redraw = true;
            }

            if redraw {
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_MOUSELEAVE => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                state.tracking_mouse = false;
                state.sidebar_hovered = None;
                state.tab_hovered = None;
                state.tab_close_hovered = None;
                state.tab_plus_hovered = false;
                state.btn_hovered = None;
                state.hovered_row = None;
                state.hovered_breadcrumb = None;
                if !state.scrollbar_dragging {
                    state.scrollbar_hovered = false;
                }
                InvalidateRect(hwnd, ptr::null(), 0);
            }
            0
        }
        WM_LBUTTONDOWN => {
            SetFocus(hwnd);
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            let x = (lparam as u32 & 0xffff) as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as i32;
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);

            let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;

            state.drag_start = None;
            state.drag_candidate = None;
            state.drag_deferred_select = false;

            // 0. Settings button click in sidebar
            let s_top = rect.bottom - CARD_MARGIN - 38;
            let s_bottom = rect.bottom - CARD_MARGIN - 8;
            if x >= CARD_MARGIN + 8
                && x < CARD_MARGIN + SIDEBAR_WIDTH - 8
                && y >= s_top
                && y < s_bottom
            {
                handle_command(hwnd, state, CMD_SETTINGS);
                return 0;
            }

            // 1. Sidebar click
            let (sb_layout, _) = compute_sidebar_layout(&state.sidebar_entries, rect.bottom);
            for item in &sb_layout {
                if x >= item.rect.left
                    && x <= item.rect.right
                    && y >= item.rect.top
                    && y <= item.rect.bottom
                {
                    if let Some(entry) = state.sidebar_entries.get(item.entry_idx) {
                        let path = entry.path.clone();
                        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                            if tab.nav.go(path) {
                                tab.search_query.clear();
                                set_control_text(hwnd, SEARCH_ID, "");
                                tab.reload(state.show_hidden);
                                tab.clear_selection();
                                tab.scroll_pos = 0;
                                set_control_text(
                                    hwnd,
                                    PATH_ID,
                                    &tab.nav.current().to_string_lossy(),
                                );
                                ensure_dir_watcher(hwnd, state);
                                update_scroll_info(hwnd, state);
                                InvalidateRect(hwnd, ptr::null(), 0);
                                return 0;
                            }
                        }
                    }
                }
            }

            // 2. Tab click or Close or Plus
            let tab_y = CARD_MARGIN + 4;
            if y >= tab_y && y < tab_y + TAB_BAR_HEIGHT - 4 {
                let mut cur_x = main_left + 10;
                for i in 0..state.tabs.len() {
                    let tab_w = 126;
                    if x >= cur_x && x < cur_x + tab_w {
                        if x >= cur_x + tab_w - 24 && x < cur_x + tab_w - 4 {
                            if state.tabs.len() > 1 {
                                state.tabs.remove(i);
                                if state.active_tab >= state.tabs.len() {
                                    state.active_tab = state.tabs.len() - 1;
                                }
                                update_controls_for_active_tab(hwnd, state);
                                ensure_dir_watcher(hwnd, state);
                                update_scroll_info(hwnd, state);
                                InvalidateRect(hwnd, ptr::null(), 0);
                            }
                        } else {
                            state.active_tab = i;
                            update_controls_for_active_tab(hwnd, state);
                            ensure_dir_watcher(hwnd, state);
                            update_scroll_info(hwnd, state);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                        return 0;
                    }
                    cur_x += tab_w + 6;
                }

                if x >= cur_x && x < cur_x + 30 {
                    let cur_path = state
                        .tabs
                        .get(state.active_tab)
                        .map(|t| t.nav.current().clone())
                        .unwrap_or_else(|| PathBuf::from("C:\\"));
                    let new_tab = Tab::new(cur_path, state.show_hidden, state.settings.view_mode);
                    state.tabs.push(new_tab);
                    state.active_tab = state.tabs.len() - 1;
                    update_controls_for_active_tab(hwnd, state);
                    ensure_dir_watcher(hwnd, state);
                    update_scroll_info(hwnd, state);
                    InvalidateRect(hwnd, ptr::null(), 0);
                    return 0;
                }
            }

            // 3. Action Buttons Click (Back, Forward, ViewDropdown)
            let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
            let btn_h = 30;
            let r_right = rect.right - CARD_MARGIN - 12;
            let view_btn_w = 76;
            let sort_btn_w = 76;
            let view_btn_x = r_right - view_btn_w;
            let sort_btn_x = view_btn_x - 8 - sort_btn_w;

            if y >= nav_y && y < nav_y + btn_h {
                if x >= main_left + 12 && x < main_left + 44 {
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        if tab.nav.back().is_some() {
                            tab.reload(state.show_hidden);
                            set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                            ensure_dir_watcher(hwnd, state);
                            update_scroll_info(hwnd, state);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
                    return 0;
                } else if x >= main_left + 48 && x < main_left + 80 {
                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        if tab.nav.forward().is_some() {
                            tab.reload(state.show_hidden);
                            set_control_text(hwnd, PATH_ID, &tab.nav.current().to_string_lossy());
                            ensure_dir_watcher(hwnd, state);
                            update_scroll_info(hwnd, state);
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
                    return 0;
                } else if x >= sort_btn_x && x < sort_btn_x + sort_btn_w {
                    handle_command(hwnd, state, CMD_SORT_DROPDOWN);
                    return 0;
                } else if x >= view_btn_x && x < view_btn_x + view_btn_w {
                    handle_command(hwnd, state, CMD_VIEW_DROPDOWN);
                    return 0;
                }
            }

            // 4. Breadcrumb Click
            if !state.is_path_editing {
                let mut clicked_crumb = false;
                for crumb in &state.breadcrumbs {
                    if x >= crumb.rect.left
                        && x <= crumb.rect.right
                        && y >= crumb.rect.top
                        && y <= crumb.rect.bottom
                    {
                        let path = crumb.path.clone();
                        if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                            if tab.nav.go(path) {
                                tab.reload(state.show_hidden);
                                set_control_text(
                                    hwnd,
                                    PATH_ID,
                                    &tab.nav.current().to_string_lossy(),
                                );
                                ensure_dir_watcher(hwnd, state);
                                update_scroll_info(hwnd, state);
                                InvalidateRect(hwnd, ptr::null(), 0);
                            }
                        }
                        clicked_crumb = true;
                        break;
                    }
                }
                if !clicked_crumb {
                    let filter_w = 120;
                    let search_x = r_right - view_btn_w - 8 - filter_w;
                    let path_x = main_left + 86;
                    let path_w = (search_x - 8 - path_x).max(120);

                    if x >= path_x && x < path_x + path_w && y >= nav_y && y < nav_y + btn_h {
                        state.is_path_editing = true;
                        SetFocus(GetDlgItem(hwnd, PATH_ID as i32));
                        SendMessageW(GetDlgItem(hwnd, PATH_ID as i32), EM_SETSEL, 0, -1);
                        InvalidateRect(hwnd, ptr::null(), 0);
                        return 0;
                    }
                } else {
                    return 0;
                }
            }

            // 5. Header Click (Sorting) - Only active in Details & Compact modes
            let is_list_view = state.tabs.get(state.active_tab).map_or(true, |t| {
                matches!(t.view_mode, ViewMode::Details | ViewMode::Compact)
            });
            if is_list_view {
                let header_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT;
                if y >= header_top && y < header_top + HEADER_HEIGHT {
                    let main_w = rect.right - CARD_MARGIN - main_left;
                    let col_type_x = main_left + (main_w * 48 / 100);
                    let col_size_x = main_left + (main_w * 64 / 100);
                    let col_date_x = main_left + (main_w * 78 / 100);

                    let clicked_col = if x < col_type_x {
                        SortColumn::Name
                    } else if x < col_size_x {
                        SortColumn::Type
                    } else if x < col_date_x {
                        SortColumn::Size
                    } else {
                        SortColumn::Date
                    };

                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        if tab.sort_column == clicked_col {
                            tab.sort_ascending = !tab.sort_ascending;
                        } else {
                            tab.sort_column = clicked_col;
                            tab.sort_ascending = true;
                        }
                        if tab.search_query.is_empty() {
                            tab.sort_current();
                        }
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                    return 0;
                }
            }

            // 5b. Scrollbar Click (Track Page Up/Down or Thumb Drag)
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let sb_metrics = calc_scrollbar_metrics(rect.right, rect.bottom, tab);
                if sb_metrics.needed
                    && x >= sb_metrics.track_left - 4
                    && x <= sb_metrics.track_right + 4
                    && y >= sb_metrics.track_top
                    && y <= sb_metrics.track_bottom
                {
                    if y >= sb_metrics.thumb_top && y <= sb_metrics.thumb_bottom {
                        state.scrollbar_dragging = true;
                        state.scrollbar_drag_offset_y = y - sb_metrics.thumb_top;
                        SetCapture(hwnd);
                    } else if y < sb_metrics.thumb_top {
                        tab.scroll_pos = (tab.scroll_pos - sb_metrics.page_items as i32).max(0);
                    } else if y > sb_metrics.thumb_bottom {
                        tab.scroll_pos = (tab.scroll_pos + sb_metrics.page_items as i32)
                            .min(sb_metrics.max_scroll);
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                    return 0;
                }
            }

            // 6. Item Click (Row or Grid Card) with Multi-Selection & Dual-Pane Support
            if state.dual_pane {
                let total_w = rect.right - CARD_MARGIN - main_left;
                let gap = 8;
                let pane_w = (total_w - gap) / 2;
                if x < main_left + pane_w {
                    switch_dual_pane(hwnd, state, 0);
                } else if x >= main_left + pane_w + gap {
                    switch_dual_pane(hwnd, state, 1);
                }
            }

            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let (pane_left, pane_right) = if state.dual_pane {
                    let total_w = rect.right - CARD_MARGIN - main_left;
                    let gap = 8;
                    let pane_w = (total_w - gap) / 2;
                    if state.dual_active == 0 {
                        (main_left, main_left + pane_w)
                    } else {
                        (main_left + pane_w + gap, rect.right - CARD_MARGIN)
                    }
                } else {
                    (main_left, rect.right - CARD_MARGIN)
                };

                let header_h = match tab.view_mode {
                    ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
                    ViewMode::Grid => 0,
                };
                let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + header_h;
                let main_bottom = rect.bottom - CARD_MARGIN - STATUS_HEIGHT;

                if x >= pane_left && x < pane_right && y >= main_top && y < main_bottom {
                    SetFocus(hwnd);
                    let ctrl_down = (GetKeyState(VK_CONTROL as i32) as u16 & 0x8000) != 0;
                    let shift_down = (GetKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0;
                    let total_count = tab.items_count();

                    let clicked_idx = match tab.view_mode {
                        ViewMode::Details => {
                            let idx = ((y - main_top) / 34) as usize + tab.scroll_pos as usize;
                            if idx < total_count {
                                Some(idx)
                            } else {
                                None
                            }
                        }
                        ViewMode::Compact => {
                            let idx = ((y - main_top) / 26) as usize + tab.scroll_pos as usize;
                            if idx < total_count {
                                Some(idx)
                            } else {
                                None
                            }
                        }
                        ViewMode::Grid => {
                            let p_w = (pane_right - pane_left - 24).max(120);
                            let cols = (p_w / 116).max(1) as usize;
                            let rel_x = x - (pane_left + 12);
                            let rel_y = y - main_top;
                            if rel_x >= 0 && rel_y >= 0 {
                                let col = (rel_x / 116) as usize;
                                let row = (rel_y / 100) as usize + tab.scroll_pos as usize;
                                if col < cols {
                                    let idx = row * cols + col;
                                    if idx < total_count {
                                        Some(idx)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    };

                    if let Some(idx) = clicked_idx {
                        if let Some(entry_idx) = tab.display_row_to_entry(idx) {
                            SetCapture(hwnd);
                            if ctrl_down {
                                tab.select_toggle(entry_idx);
                                state.drag_start = Some(POINT { x, y });
                                state.drag_candidate = Some(entry_idx);
                                state.drag_deferred_select = false;
                            } else if shift_down {
                                tab.select_range(entry_idx);
                                state.drag_start = Some(POINT { x, y });
                                state.drag_candidate = Some(entry_idx);
                                state.drag_deferred_select = false;
                            } else if tab.is_selected(entry_idx) {
                                state.drag_start = Some(POINT { x, y });
                                state.drag_candidate = Some(entry_idx);
                                state.drag_deferred_select = true;
                            } else {
                                tab.select_single(entry_idx);
                                state.drag_start = Some(POINT { x, y });
                                state.drag_candidate = Some(entry_idx);
                                state.drag_deferred_select = false;
                            }
                            if let Some(item) = tab.selected_item() {
                                preview::update_preview_if_open(&item.path);
                            }
                        }
                    } else if !ctrl_down && !shift_down {
                        state.drag_start = None;
                        state.drag_candidate = None;
                        state.drag_deferred_select = false;
                        ReleaseCapture();
                        tab.clear_selection();
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_LBUTTONUP => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                ReleaseCapture();
                if state.scrollbar_dragging {
                    state.scrollbar_dragging = false;
                    InvalidateRect(hwnd, ptr::null(), 0);
                    return 0;
                }
                if state.drag_start.is_some() {
                    state.drag_start = None;
                    if state.drag_deferred_select {
                        state.drag_deferred_select = false;
                        if let Some(entry_idx) = state.drag_candidate.take() {
                            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                                tab.select_single(entry_idx);
                                if let Some(item) = tab.selected_item() {
                                    preview::update_preview_if_open(&item.path);
                                }
                                InvalidateRect(hwnd, ptr::null(), 0);
                            }
                        }
                    } else {
                        state.drag_candidate = None;
                    }
                }
            }
            0
        }
        WM_CAPTURECHANGED => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                state.drag_start = None;
                state.drag_candidate = None;
                state.drag_deferred_select = false;
                if state.scrollbar_dragging {
                    state.scrollbar_dragging = false;
                    InvalidateRect(hwnd, ptr::null(), 0);
                }
            }
            0
        }
        WM_RBUTTONUP => {
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            let x = (lparam as u32 & 0xffff) as i32;
            let y = ((lparam as u32 >> 16) & 0xffff) as i32;
            let mut rect: RECT = zeroed();
            GetClientRect(hwnd, &mut rect);

            let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;
            let mut pt = POINT { x, y };
            ClientToScreen(hwnd, &mut pt);

            // 1. Right Click on Sidebar
            let (sb_layout, _) = compute_sidebar_layout(&state.sidebar_entries, rect.bottom);
            for item in &sb_layout {
                if x >= item.rect.left
                    && x <= item.rect.right
                    && y >= item.rect.top
                    && y <= item.rect.bottom
                {
                    show_sidebar_context_menu(hwnd, state, pt, item.entry_idx);
                    return 0;
                }
            }

            // 2. Right Click on Tabs
            let tab_y = CARD_MARGIN + 4;
            if y >= tab_y && y < tab_y + TAB_BAR_HEIGHT - 4 && x >= main_left + 10 {
                let mut cur_x = main_left + 10;
                for i in 0..state.tabs.len() {
                    let tab_w = 130;
                    if x >= cur_x && x < cur_x + tab_w {
                        show_tab_context_menu(hwnd, state, pt, i);
                        return 0;
                    }
                    cur_x += tab_w + 6;
                }
            }

            // 3. Right Click on Main Canvas
            if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                let header_h = match tab.view_mode {
                    ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
                    ViewMode::Grid => 0,
                };
                let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + header_h;
                let main_bottom = rect.bottom - CARD_MARGIN - STATUS_HEIGHT;

                if x >= main_left
                    && x < rect.right - CARD_MARGIN
                    && y >= main_top
                    && y < main_bottom
                {
                    let total_count = tab.items_count();
                    let clicked_idx = match tab.view_mode {
                        ViewMode::Details => {
                            let idx = ((y - main_top) / 34) as usize + tab.scroll_pos as usize;
                            if idx < total_count {
                                Some(idx)
                            } else {
                                None
                            }
                        }
                        ViewMode::Compact => {
                            let idx = ((y - main_top) / 26) as usize + tab.scroll_pos as usize;
                            if idx < total_count {
                                Some(idx)
                            } else {
                                None
                            }
                        }
                        ViewMode::Grid => {
                            let main_w = (rect.right - CARD_MARGIN - main_left - 24).max(120);
                            let cols = (main_w / 116).max(1) as usize;
                            let rel_x = x - (main_left + 12);
                            let rel_y = y - main_top;
                            if rel_x >= 0 && rel_y >= 0 {
                                let col = (rel_x / 116) as usize;
                                let row = (rel_y / 100) as usize + tab.scroll_pos as usize;
                                if col < cols {
                                    let idx = row * cols + col;
                                    if idx < total_count {
                                        Some(idx)
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        }
                    };

                    let mut clicked_item = false;
                    if let Some(idx) = clicked_idx {
                        if let Some(entry_idx) = tab.display_row_to_entry(idx) {
                            if !tab.is_selected(entry_idx) {
                                tab.select_single(entry_idx);
                            }
                            clicked_item = true;
                        }
                    }
                    InvalidateRect(hwnd, ptr::null(), 0);
                    show_context_menu(hwnd, state, pt, clicked_item);
                }
            }
            0
        }
        WM_LBUTTONDBLCLK => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                let x = (lparam as u32 & 0xffff) as i32;
                let y = ((lparam as u32 >> 16) & 0xffff) as i32;
                let mut rect: RECT = zeroed();
                GetClientRect(hwnd, &mut rect);
                let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;

                if state.dual_pane {
                    let total_w = rect.right - CARD_MARGIN - main_left;
                    let gap = 8;
                    let pane_w = (total_w - gap) / 2;
                    if x < main_left + pane_w {
                        switch_dual_pane(hwnd, state, 0);
                    } else if x >= main_left + pane_w + gap {
                        switch_dual_pane(hwnd, state, 1);
                    }
                }

                let mut on_item = false;
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    let (pane_left, pane_right) = if state.dual_pane {
                        let total_w = rect.right - CARD_MARGIN - main_left;
                        let gap = 8;
                        let pane_w = (total_w - gap) / 2;
                        if state.dual_active == 0 {
                            (main_left, main_left + pane_w)
                        } else {
                            (main_left + pane_w + gap, rect.right - CARD_MARGIN)
                        }
                    } else {
                        (main_left, rect.right - CARD_MARGIN)
                    };

                    let header_h = match tab.view_mode {
                        ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
                        ViewMode::Grid => 0,
                    };
                    let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + header_h;
                    let main_bottom = rect.bottom - CARD_MARGIN - STATUS_HEIGHT;

                    if x >= pane_left && x < pane_right && y >= main_top && y < main_bottom {
                        let total_count = tab.items_count();
                        let clicked_idx = match tab.view_mode {
                            ViewMode::Details => {
                                let idx = ((y - main_top) / 34) as usize + tab.scroll_pos as usize;
                                if idx < total_count {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            ViewMode::Compact => {
                                let idx = ((y - main_top) / 26) as usize + tab.scroll_pos as usize;
                                if idx < total_count {
                                    Some(idx)
                                } else {
                                    None
                                }
                            }
                            ViewMode::Grid => {
                                let p_w = (pane_right - pane_left - 24).max(120);
                                let cols = (p_w / 116).max(1) as usize;
                                let rel_x = x - (pane_left + 12);
                                let rel_y = y - main_top;
                                if rel_x >= 0 && rel_y >= 0 {
                                    let col = (rel_x / 116) as usize;
                                    let row = (rel_y / 100) as usize + tab.scroll_pos as usize;
                                    if col < cols {
                                        let idx = row * cols + col;
                                        if idx < total_count {
                                            Some(idx)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            }
                        };
                        if let Some(idx) = clicked_idx {
                            if let Some(entry_idx) = tab.display_row_to_entry(idx) {
                                tab.select_single(entry_idx);
                                on_item = true;
                            }
                        }
                    }
                }

                if on_item {
                    state.drag_start = None;
                    state.drag_candidate = None;
                    state.drag_deferred_select = false;
                    ReleaseCapture();
                    open_selected_item(hwnd, state);
                }
            }
            0
        }
        WM_MOUSEWHEEL => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                let delta = ((wparam >> 16) & 0xffff) as i16;
                let mut rect: RECT = zeroed();
                GetClientRect(hwnd, &mut rect);
                if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                    let sb_metrics = calc_scrollbar_metrics(rect.right, rect.bottom, tab);
                    if sb_metrics.needed {
                        let lines = (delta as i32 / 120) * 3;
                        let new_scroll = (tab.scroll_pos - lines).clamp(0, sb_metrics.max_scroll);
                        if new_scroll != tab.scroll_pos {
                            tab.scroll_pos = new_scroll;
                            InvalidateRect(hwnd, ptr::null(), 0);
                        }
                    }
                }
            }
            0
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            if state_ptr.is_null() {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            let state = &*state_ptr;
            let dc = wparam as HDC;
            SetTextColor(dc, CLR_PRIMARY_TEXT);
            SetBkColor(dc, CLR_INPUT_BG);
            SetBkMode(dc, TRANSPARENT as i32);
            state.brush_input as LRESULT
        }
        WM_DROPFILES => {
            if state_ptr.is_null() {
                return 0;
            }
            let state = &mut *state_ptr;
            let hdrop = wparam as HDROP;
            let file_count = DragQueryFileW(hdrop, 0xFFFFFFFF, ptr::null_mut(), 0);
            if file_count > 0 {
                let mut pt: POINT = zeroed();
                DragQueryPoint(hdrop, &mut pt);

                let mut rect: RECT = zeroed();
                GetClientRect(hwnd, &mut rect);
                let (sb_layout, _) = compute_sidebar_layout(&state.sidebar_entries, rect.bottom);

                let mut target_dir = None;
                for item in &sb_layout {
                    if pt.x >= item.rect.left
                        && pt.x <= item.rect.right
                        && pt.y >= item.rect.top
                        && pt.y <= item.rect.bottom
                    {
                        if let Some(entry) = state.sidebar_entries.get(item.entry_idx) {
                            target_dir = Some(entry.path.clone());
                            break;
                        }
                    }
                }

                let target = target_dir.unwrap_or_else(|| {
                    state
                        .tabs
                        .get(state.active_tab)
                        .map(|t| t.nav.current().clone())
                        .unwrap_or_else(|| PathBuf::from("C:\\"))
                });

                let mut src_paths: Vec<PathBuf> = Vec::new();
                for i in 0..file_count {
                    let mut buf = [0u16; 1024];
                    let len = DragQueryFileW(hdrop, i, buf.as_mut_ptr(), buf.len() as u32);
                    if len > 0 {
                        let path_str = String::from_utf16_lossy(&buf[..len as usize]);
                        src_paths.push(PathBuf::from(path_str));
                    }
                }

                if !src_paths.is_empty() {
                    let mut from_buf: Vec<u16> = Vec::new();
                    for p in &src_paths {
                        let ws = wide(&p.to_string_lossy());
                        from_buf.extend_from_slice(&ws);
                    }
                    from_buf.push(0);

                    let mut to_buf = wide(&target.to_string_lossy());
                    to_buf.push(0);

                    let mut op: SHFILEOPSTRUCTW = zeroed();
                    op.hwnd = hwnd;
                    op.wFunc = FO_COPY;
                    op.pFrom = from_buf.as_ptr();
                    op.pTo = to_buf.as_ptr();
                    op.fFlags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION) as u16;

                    SHFileOperationW(&mut op);

                    if let Some(tab) = state.tabs.get_mut(state.active_tab) {
                        tab.reload(state.show_hidden);
                        update_scroll_info(hwnd, state);
                        InvalidateRect(hwnd, ptr::null(), 0);
                    }
                }
            }
            DragFinish(hdrop);
            0
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                let state = &mut *state_ptr;
                if let Some(ref w) = state.dir_watcher {
                    w.stop();
                }
                state.settings.last_session = state
                    .tabs
                    .iter()
                    .map(|t| t.nav.current().to_string_lossy().to_string())
                    .collect();
                let _ = state.settings.save();
            }
            KillTimer(hwnd, 1);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn draw_vector_pinned_folder(dc: HDC, x: i32, y: i32, color: COLORREF) {
    draw_vector_folder(dc, x, y, color);
    unsafe {
        let pin_brush = CreateSolidBrush(CLR_ACCENT);
        let pen = CreatePen(PS_NULL as i32, 0, 0);
        let old_b = SelectObject(dc, pin_brush);
        let old_p = SelectObject(dc, pen);
        Ellipse(dc, x + 11, y + 2, x + 16, y + 7);
        SelectObject(dc, old_b);
        SelectObject(dc, old_p);
        DeleteObject(pin_brush);
        DeleteObject(pen);
    }
}

fn draw_vector_folder_large(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x, y, x + 24, y + 14, 4, 4);
        RoundRect(dc, x, y + 7, x + 50, y + 42, 6, 6);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

fn draw_vector_file_large(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x + 5, y, x + 42, y + 44, 5, 5);
        let fold_brush = CreateSolidBrush(CLR_MAIN_CARD_BG);
        let old_fb = SelectObject(dc, fold_brush);
        RoundRect(dc, x + 28, y, x + 42, y + 14, 3, 3);
        SelectObject(dc, old_fb);
        DeleteObject(fold_brush);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

pub(crate) fn draw_vector_folder(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x, y, x + 7, y + 6, 2, 2);
        RoundRect(dc, x, y + 3, x + 16, y + 14, 3, 3);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

pub(crate) fn draw_vector_file(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x + 1, y, x + 14, y + 15, 2, 2);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

pub(crate) fn draw_vector_drive(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x, y + 2, x + 16, y + 13, 3, 3);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

fn draw_vector_brand_logo(dc: HDC, x: i32, y: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        let pen = CreatePen(PS_SOLID as i32, 1, color);
        let old_brush = SelectObject(dc, brush);
        let old_pen = SelectObject(dc, pen);

        RoundRect(dc, x, y, x + 9, y + 9, 3, 3);
        RoundRect(dc, x + 5, y + 5, x + 14, y + 14, 3, 3);

        SelectObject(dc, old_pen);
        SelectObject(dc, old_brush);
        DeleteObject(pen);
        DeleteObject(brush);
    }
}

unsafe fn draw_highlighted_text(
    dc: HDC,
    name_w: &[u16],
    query: &str,
    rc: &RECT,
    color_normal: COLORREF,
    color_highlight: COLORREF,
    flags: u32,
) {
    if query.is_empty() {
        SetTextColor(dc, color_normal);
        let mut w = name_w.to_vec();
        w.push(0);
        let mut r = *rc;
        DrawTextW(dc, w.as_ptr(), (w.len() - 1) as i32, &mut r, flags);
        return;
    }

    let name_str = String::from_utf16_lossy(name_w);
    let lower_name = name_str.to_lowercase();
    let lower_query = query.to_lowercase();

    if let Some(match_idx) = lower_name.find(&lower_query) {
        let match_end = match_idx + lower_query.len();
        let prefix = &name_str[..match_idx];
        let matched = &name_str[match_idx..match_end];
        let suffix = &name_str[match_end..];

        let mut cur_left = rc.left;

        // Prefix
        if !prefix.is_empty() {
            SetTextColor(dc, color_normal);
            let w = wide(prefix);
            let mut sz: windows_sys::Win32::Foundation::SIZE = zeroed();
            GetTextExtentPoint32W(dc, w.as_ptr(), (w.len() - 1) as i32, &mut sz);
            let mut r = RECT {
                left: cur_left,
                top: rc.top,
                right: (cur_left + sz.cx).min(rc.right),
                bottom: rc.bottom,
            };
            DrawTextW(dc, w.as_ptr(), (w.len() - 1) as i32, &mut r, flags);
            cur_left += sz.cx;
        }

        // Matched portion with neon accent color
        if cur_left < rc.right && !matched.is_empty() {
            SetTextColor(dc, color_highlight);
            let w = wide(matched);
            let mut sz: windows_sys::Win32::Foundation::SIZE = zeroed();
            GetTextExtentPoint32W(dc, w.as_ptr(), (w.len() - 1) as i32, &mut sz);
            let mut r = RECT {
                left: cur_left,
                top: rc.top,
                right: (cur_left + sz.cx).min(rc.right),
                bottom: rc.bottom,
            };
            DrawTextW(dc, w.as_ptr(), (w.len() - 1) as i32, &mut r, flags);
            cur_left += sz.cx;
        }

        // Suffix
        if cur_left < rc.right && !suffix.is_empty() {
            SetTextColor(dc, color_normal);
            let w = wide(suffix);
            let mut r = RECT {
                left: cur_left,
                top: rc.top,
                right: rc.right,
                bottom: rc.bottom,
            };
            DrawTextW(dc, w.as_ptr(), (w.len() - 1) as i32, &mut r, flags);
        }
    } else {
        SetTextColor(dc, color_normal);
        let mut w = name_w.to_vec();
        w.push(0);
        let mut r = *rc;
        DrawTextW(dc, w.as_ptr(), (w.len() - 1) as i32, &mut r, flags);
    }
}

unsafe fn paint(hwnd: HWND, state: &mut State) {
    let mut ps: PAINTSTRUCT = zeroed();
    let target_dc = BeginPaint(hwnd, &mut ps);
    let mut rect: RECT = zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right;
    let height = rect.bottom;

    if width <= 0 || height <= 0 {
        EndPaint(hwnd, &ps);
        return;
    }

    let mem_dc = CreateCompatibleDC(target_dc);
    let mem_bmp = CreateCompatibleBitmap(target_dc, width, height);
    let old_bmp = SelectObject(mem_dc, mem_bmp);

    // 1. Soft Deep Outer Canvas
    FillRect(mem_dc, &rect, state.brush_outer);

    // 2. Left Sidebar Card (Rounded Capsule Container)
    let sidebar_rect = RECT {
        left: CARD_MARGIN,
        top: CARD_MARGIN,
        right: CARD_MARGIN + SIDEBAR_WIDTH,
        bottom: height - CARD_MARGIN,
    };
    SelectObject(mem_dc, state.pen_card_border);
    SelectObject(mem_dc, state.brush_sidebar);
    RoundRect(
        mem_dc,
        sidebar_rect.left,
        sidebar_rect.top,
        sidebar_rect.right,
        sidebar_rect.bottom,
        CARD_RADIUS,
        CARD_RADIUS,
    );

    // Sidebar App Branding with Vector Icon
    let brand_top = CARD_MARGIN + 16;
    draw_vector_brand_logo(mem_dc, CARD_MARGIN + 16, brand_top + 4, CLR_ACCENT);

    SelectObject(mem_dc, state.font_header);
    SetTextColor(mem_dc, CLR_PRIMARY_TEXT);
    SetBkMode(mem_dc, TRANSPARENT as i32);
    let brand_name = wide("ROVIK");
    let mut brand_rect = RECT {
        left: CARD_MARGIN + 46,
        top: brand_top + 3,
        right: CARD_MARGIN + SIDEBAR_WIDTH - 12,
        bottom: brand_top + 28,
    };
    DrawTextW(
        mem_dc,
        brand_name.as_ptr(),
        (brand_name.len() - 1) as i32,
        &mut brand_rect,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );

    // Quick Access Section Title
    let qa_title_y = CARD_MARGIN + 56;
    SelectObject(mem_dc, state.font_small);
    SetTextColor(mem_dc, CLR_MUTED_TEXT);
    let qa_text = wide("QUICK ACCESS");
    let mut qa_rect = RECT {
        left: CARD_MARGIN + 16,
        top: qa_title_y,
        right: CARD_MARGIN + SIDEBAR_WIDTH - 12,
        bottom: qa_title_y + 18,
    };
    DrawTextW(
        mem_dc,
        qa_text.as_ptr(),
        (qa_text.len() - 1) as i32,
        &mut qa_rect,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );

    // Sidebar Items
    SelectObject(mem_dc, state.font_normal);
    let cur_active_path = state
        .tabs
        .get(state.active_tab)
        .map(|t| t.nav.current().clone());
    let (sb_layout, drives_header_y) = compute_sidebar_layout(&state.sidebar_entries, height);

    // Render DRIVES section title if present
    if let Some(d_y) = drives_header_y {
        SelectObject(mem_dc, state.font_small);
        SetTextColor(mem_dc, CLR_MUTED_TEXT);
        let drives_text = wide("DRIVES");
        let mut d_rect = RECT {
            left: CARD_MARGIN + 16,
            top: d_y,
            right: CARD_MARGIN + SIDEBAR_WIDTH - 12,
            bottom: d_y + 18,
        };
        DrawTextW(
            mem_dc,
            drives_text.as_ptr(),
            (drives_text.len() - 1) as i32,
            &mut d_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
        SelectObject(mem_dc, state.font_normal);
    }

    for item in &sb_layout {
        let entry = &state.sidebar_entries[item.entry_idx];
        let is_active = cur_active_path.as_ref().map_or(false, |p| *p == entry.path);
        let is_hovered = state.sidebar_hovered == Some(item.entry_idx);
        let item_pill = item.rect;

        SelectObject(mem_dc, state.pen_null);
        if is_active {
            SelectObject(mem_dc, state.brush_select);
            RoundRect(
                mem_dc,
                item_pill.left,
                item_pill.top,
                item_pill.right,
                item_pill.bottom,
                PILL_RADIUS,
                PILL_RADIUS,
            );

            // Left indicator pill accent
            let accent_bar = RECT {
                left: item_pill.left + 3,
                top: item_pill.top + 6,
                right: item_pill.left + 6,
                bottom: item_pill.bottom - 6,
            };
            FillRect(mem_dc, &accent_bar, state.brush_accent);
        } else if is_hovered {
            SelectObject(mem_dc, state.brush_hover);
            RoundRect(
                mem_dc,
                item_pill.left,
                item_pill.top,
                item_pill.right,
                item_pill.bottom,
                PILL_RADIUS,
                PILL_RADIUS,
            );
        }

        if entry.is_drive {
            // Drive vector icon
            let icon_x = item_pill.left + 10;
            let icon_y = item_pill.top + 7;
            draw_vector_drive(
                mem_dc,
                icon_x,
                icon_y,
                if is_active {
                    CLR_ACCENT
                } else {
                    CLR_SECONDARY_TEXT
                },
            );

            // Drive Title (Local Disk (C:))
            SetTextColor(
                mem_dc,
                if is_active {
                    CLR_PRIMARY_TEXT
                } else {
                    CLR_SECONDARY_TEXT
                },
            );
            SelectObject(mem_dc, state.font_normal);
            let w_title = wide(&entry.title);
            let mut text_rect = RECT {
                left: item_pill.left + 34,
                top: item_pill.top + 4,
                right: item_pill.right - 8,
                bottom: item_pill.top + 20,
            };
            DrawTextW(
                mem_dc,
                w_title.as_ptr(),
                (w_title.len() - 1) as i32,
                &mut text_rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            // Storage usage bar and detailed text
            if let (Some(total), Some(free)) = (entry.total_bytes, entry.free_bytes) {
                let used = total.saturating_sub(free);
                let ratio = if total > 0 {
                    (used as f32 / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };

                let bar_left = item_pill.left + 28;
                let bar_right = item_pill.right - 8;
                let bar_top = item_pill.top + 23;
                let bar_bottom = bar_top + 5;

                let bar_rect = RECT {
                    left: bar_left,
                    top: bar_top,
                    right: bar_right,
                    bottom: bar_bottom,
                };
                SelectObject(mem_dc, state.brush_hover);
                RoundRect(
                    mem_dc,
                    bar_rect.left,
                    bar_rect.top,
                    bar_rect.right,
                    bar_rect.bottom,
                    2,
                    2,
                );

                let fill_w = (((bar_right - bar_left) as f32) * ratio) as i32;
                if fill_w > 0 {
                    let fill_rect = RECT {
                        left: bar_left,
                        top: bar_top,
                        right: (bar_left + fill_w).min(bar_right),
                        bottom: bar_bottom,
                    };
                    let fill_brush = if ratio > 0.9 {
                        CreateSolidBrush(CLR_ERROR_TEXT)
                    } else {
                        CreateSolidBrush(CLR_ACCENT)
                    };
                    FillRect(mem_dc, &fill_rect, fill_brush);
                    DeleteObject(fill_brush);
                }

                SelectObject(mem_dc, state.font_small);

                // Line 1: "Terpakai: 195.3 GB / 475.8 GB"
                SetTextColor(mem_dc, CLR_PRIMARY_TEXT);
                let line1 = format!(
                    "Terpakai: {} / {}",
                    format_disk_size(used),
                    format_disk_size(total)
                );
                let w_l1 = wide(&line1);
                let mut l1_rect = RECT {
                    left: bar_left,
                    top: bar_bottom + 3,
                    right: bar_right,
                    bottom: bar_bottom + 16,
                };
                DrawTextW(
                    mem_dc,
                    w_l1.as_ptr(),
                    (w_l1.len() - 1) as i32,
                    &mut l1_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                );

                // Line 2: "Tersedia: 280.6 GB free"
                SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
                let line2 = format!("Tersedia: {} free", format_disk_size(free));
                let w_l2 = wide(&line2);
                let mut l2_rect = RECT {
                    left: bar_left,
                    top: bar_bottom + 16,
                    right: bar_right,
                    bottom: item_pill.bottom - 2,
                };
                DrawTextW(
                    mem_dc,
                    w_l2.as_ptr(),
                    (w_l2.len() - 1) as i32,
                    &mut l2_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                );
                SelectObject(mem_dc, state.font_normal);
            }
        } else {
            // Normal Folder / Pinned Item
            let icon_x = CARD_MARGIN + 20;
            let icon_y = item_pill.top + 8;
            if entry.is_pinned {
                draw_vector_pinned_folder(
                    mem_dc,
                    icon_x,
                    icon_y,
                    if is_active {
                        CLR_ACCENT
                    } else {
                        CLR_SECONDARY_TEXT
                    },
                );
            } else {
                draw_vector_folder(
                    mem_dc,
                    icon_x,
                    icon_y,
                    if is_active {
                        CLR_ACCENT
                    } else {
                        CLR_SECONDARY_TEXT
                    },
                );
            }

            SetTextColor(
                mem_dc,
                if is_active {
                    CLR_PRIMARY_TEXT
                } else {
                    CLR_SECONDARY_TEXT
                },
            );
            let w_title = wide(&entry.title);
            let mut text_rect = RECT {
                left: CARD_MARGIN + 42,
                top: item_pill.top,
                right: CARD_MARGIN + SIDEBAR_WIDTH - 14,
                bottom: item_pill.bottom,
            };
            DrawTextW(
                mem_dc,
                w_title.as_ptr(),
                (w_title.len() - 1) as i32,
                &mut text_rect,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    }

    // Settings Button at Sidebar Bottom
    let s_top = height - CARD_MARGIN - 38;
    let s_rect = RECT {
        left: CARD_MARGIN + 8,
        top: s_top,
        right: CARD_MARGIN + SIDEBAR_WIDTH - 8,
        bottom: s_top + 30,
    };
    SelectObject(mem_dc, state.pen_null);
    if state.settings_btn_hovered {
        SelectObject(mem_dc, state.brush_hover);
        RoundRect(
            mem_dc,
            s_rect.left,
            s_rect.top,
            s_rect.right,
            s_rect.bottom,
            PILL_RADIUS,
            PILL_RADIUS,
        );
    }
    SelectObject(mem_dc, state.font_normal);
    SetTextColor(
        mem_dc,
        if state.settings_btn_hovered {
            CLR_PRIMARY_TEXT
        } else {
            CLR_SECONDARY_TEXT
        },
    );
    let s_w = wide("⚙  Settings");
    let mut s_dr = s_rect;
    DrawTextW(
        mem_dc,
        s_w.as_ptr(),
        (s_w.len() - 1) as i32,
        &mut s_dr,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
    );

    // 3. Right Main Area Card (Rounded Container)
    let main_left = CARD_MARGIN + SIDEBAR_WIDTH + CARD_MARGIN;
    let main_rect = RECT {
        left: main_left,
        top: CARD_MARGIN,
        right: width - CARD_MARGIN,
        bottom: height - CARD_MARGIN,
    };
    SelectObject(mem_dc, state.pen_card_border);
    SelectObject(mem_dc, state.brush_main_card);
    RoundRect(
        mem_dc,
        main_rect.left,
        main_rect.top,
        main_rect.right,
        main_rect.bottom,
        CARD_RADIUS,
        CARD_RADIUS,
    );

    // 4. Tab Bar (Inside Main Card)
    let tab_y = CARD_MARGIN + 4;
    let mut cur_tab_x = main_left + 10;

    for (i, tab) in state.tabs.iter().enumerate() {
        let is_active = i == state.active_tab;
        let is_hovered = state.tab_hovered == Some(i);
        let tab_w = 126;

        let tab_rect = RECT {
            left: cur_tab_x,
            top: tab_y,
            right: cur_tab_x + tab_w,
            bottom: tab_y + TAB_BAR_HEIGHT - 6,
        };

        SelectObject(mem_dc, state.pen_null);
        if is_active {
            SelectObject(mem_dc, state.brush_hover);
            RoundRect(
                mem_dc,
                tab_rect.left,
                tab_rect.top,
                tab_rect.right,
                tab_rect.bottom,
                PILL_RADIUS,
                PILL_RADIUS,
            );
            let tab_indicator = RECT {
                left: tab_rect.left + 16,
                top: tab_rect.bottom - 2,
                right: tab_rect.right - 16,
                bottom: tab_rect.bottom,
            };
            FillRect(mem_dc, &tab_indicator, state.brush_accent);
        } else if is_hovered {
            SelectObject(mem_dc, state.brush_tab_inactive);
            RoundRect(
                mem_dc,
                tab_rect.left,
                tab_rect.top,
                tab_rect.right,
                tab_rect.bottom,
                PILL_RADIUS,
                PILL_RADIUS,
            );
        }

        SetTextColor(
            mem_dc,
            if is_active {
                CLR_PRIMARY_TEXT
            } else {
                CLR_SECONDARY_TEXT
            },
        );
        SelectObject(mem_dc, state.font_normal);
        let tab_title = wide(&tab.title());
        let mut title_rect = RECT {
            left: tab_rect.left + 12,
            top: tab_rect.top,
            right: tab_rect.right - 26,
            bottom: tab_rect.bottom,
        };
        DrawTextW(
            mem_dc,
            tab_title.as_ptr(),
            (tab_title.len() - 1) as i32,
            &mut title_rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );

        // Close Button (×)
        if state.tabs.len() > 1 {
            let is_close_hover = state.tab_close_hovered == Some(i);
            SetTextColor(
                mem_dc,
                if is_close_hover {
                    CLR_ACCENT
                } else {
                    CLR_MUTED_TEXT
                },
            );
            let close_str = wide("×");
            let mut close_rect = RECT {
                left: tab_rect.right - 24,
                top: tab_rect.top,
                right: tab_rect.right - 6,
                bottom: tab_rect.bottom,
            };
            DrawTextW(
                mem_dc,
                close_str.as_ptr(),
                (close_str.len() - 1) as i32,
                &mut close_rect,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
            );
        }

        cur_tab_x += tab_w + 6;
    }

    // New Tab Plus Button (+)
    let plus_rect = RECT {
        left: cur_tab_x,
        top: tab_y,
        right: cur_tab_x + 28,
        bottom: tab_y + TAB_BAR_HEIGHT - 6,
    };
    SelectObject(mem_dc, state.pen_null);
    if state.tab_plus_hovered {
        SelectObject(mem_dc, state.brush_hover);
        RoundRect(
            mem_dc,
            plus_rect.left,
            plus_rect.top,
            plus_rect.right,
            plus_rect.bottom,
            PILL_RADIUS,
            PILL_RADIUS,
        );
    }
    SetTextColor(
        mem_dc,
        if state.tab_plus_hovered {
            CLR_PRIMARY_TEXT
        } else {
            CLR_SECONDARY_TEXT
        },
    );
    let plus_str = wide("+");
    let mut pr = plus_rect;
    DrawTextW(
        mem_dc,
        plus_str.as_ptr(),
        (plus_str.len() - 1) as i32,
        &mut pr,
        DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
    );

    // 5. Navigation & Action Bar
    let nav_y = CARD_MARGIN + TAB_BAR_HEIGHT + 7;
    let btn_h = 30;

    let draw_btn_pill = |dc: HDC, rect: RECT, text: &str, is_hover: bool, accent: bool| {
        SelectObject(dc, state.pen_null);
        if accent {
            SelectObject(dc, state.brush_select);
        } else if is_hover {
            SelectObject(dc, state.brush_hover);
        } else {
            SelectObject(dc, state.brush_btn);
        }
        RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            PILL_RADIUS,
            PILL_RADIUS,
        );

        SetTextColor(
            dc,
            if accent {
                CLR_ACCENT
            } else if is_hover {
                CLR_PRIMARY_TEXT
            } else {
                CLR_SECONDARY_TEXT
            },
        );
        let w = wide(text);
        let mut r = rect;
        DrawTextW(
            dc,
            w.as_ptr(),
            (w.len() - 1) as i32,
            &mut r,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
        );
    };

    // Navigation buttons (←, →)
    let b_back = RECT {
        left: main_left + 12,
        top: nav_y,
        right: main_left + 44,
        bottom: nav_y + btn_h,
    };
    let b_fwd = RECT {
        left: main_left + 48,
        top: nav_y,
        right: main_left + 80,
        bottom: nav_y + btn_h,
    };

    draw_btn_pill(
        mem_dc,
        b_back,
        "←",
        state.btn_hovered == Some(TopButton::Back),
        false,
    );
    draw_btn_pill(
        mem_dc,
        b_fwd,
        "→",
        state.btn_hovered == Some(TopButton::Forward),
        false,
    );

    // View & Sort Dropdown Buttons at top right
    let r_right = width - CARD_MARGIN - 12;
    let view_btn_w = 76;
    let sort_btn_w = 76;
    let view_btn_x = r_right - view_btn_w;
    let sort_btn_x = view_btn_x - 8 - sort_btn_w;

    let b_view = RECT {
        left: view_btn_x,
        top: nav_y,
        right: view_btn_x + view_btn_w,
        bottom: nav_y + btn_h,
    };
    draw_btn_pill(
        mem_dc,
        b_view,
        "☷ View ▾",
        state.btn_hovered == Some(TopButton::ViewDropdown),
        false,
    );

    let b_sort = RECT {
        left: sort_btn_x,
        top: nav_y,
        right: sort_btn_x + sort_btn_w,
        bottom: nav_y + btn_h,
    };
    let (cur_col, cur_asc) = state
        .tabs
        .get(state.active_tab)
        .map(|t| (t.sort_column, t.sort_ascending))
        .unwrap_or((SortColumn::Name, true));
    let arrow = if cur_asc { "▲" } else { "▼" };
    let sort_title = match cur_col {
        SortColumn::Name => "Name",
        SortColumn::Date => "Date",
        SortColumn::Size => "Size",
        SortColumn::Type => "Type",
    };
    let sort_label = format!("{arrow} {sort_title} ▾");
    draw_btn_pill(
        mem_dc,
        b_sort,
        &sort_label,
        state.btn_hovered == Some(TopButton::SortDropdown),
        false,
    );

    // Filter & Path Pills
    let filter_w = 110;
    let search_x = sort_btn_x - 8 - filter_w;
    let path_x = main_left + 86;
    let path_w = (search_x - 8 - path_x).max(100);

    let path_pill = RECT {
        left: path_x,
        top: nav_y,
        right: path_x + path_w,
        bottom: nav_y + btn_h,
    };
    let search_pill = RECT {
        left: search_x,
        top: nav_y,
        right: search_x + filter_w,
        bottom: nav_y + btn_h,
    };

    SelectObject(mem_dc, state.pen_card_border);
    SelectObject(mem_dc, state.brush_input);
    RoundRect(
        mem_dc,
        path_pill.left,
        path_pill.top,
        path_pill.right,
        path_pill.bottom,
        PILL_RADIUS,
        PILL_RADIUS,
    );
    RoundRect(
        mem_dc,
        search_pill.left,
        search_pill.top,
        search_pill.right,
        search_pill.bottom,
        PILL_RADIUS,
        PILL_RADIUS,
    );

    // Interactive Breadcrumbs
    state.breadcrumbs.clear();
    let path_edit_hwnd = GetDlgItem(hwnd, PATH_ID as i32);

    if !state.is_path_editing {
        ShowWindow(path_edit_hwnd, SW_HIDE);
        if let Some(tab) = state.tabs.get(state.active_tab) {
            let cur_path = tab.nav.current();
            let mut crumbs = Vec::new();
            let mut accum = PathBuf::new();

            for comp in cur_path.components() {
                accum.push(comp.as_os_str());
                let title = match comp {
                    std::path::Component::Prefix(p) => p.as_os_str().to_string_lossy().to_string(),
                    std::path::Component::RootDir => continue,
                    other => other.as_os_str().to_string_lossy().to_string(),
                };
                crumbs.push((title, accum.clone()));
            }

            SelectObject(mem_dc, state.font_normal);
            let mut bc_x = path_pill.left + 10;
            for (idx, (crumb_title, crumb_path)) in crumbs.into_iter().enumerate() {
                let disp = format!("{crumb_title}  ›");
                let dw = wide(&disp);
                let crumb_w = (crumb_title.len() as i32 * 8) + 20;

                if bc_x + crumb_w > path_pill.right - 8 {
                    break;
                }

                let c_rect = RECT {
                    left: bc_x,
                    top: path_pill.top + 3,
                    right: bc_x + crumb_w,
                    bottom: path_pill.bottom - 3,
                };

                let is_bc_hover = state.hovered_breadcrumb == Some(idx);
                if is_bc_hover {
                    SelectObject(mem_dc, state.pen_null);
                    SelectObject(mem_dc, state.brush_hover);
                    RoundRect(
                        mem_dc,
                        c_rect.left,
                        c_rect.top,
                        c_rect.right,
                        c_rect.bottom,
                        6,
                        6,
                    );
                }

                SetTextColor(
                    mem_dc,
                    if is_bc_hover {
                        CLR_PRIMARY_TEXT
                    } else {
                        CLR_SECONDARY_TEXT
                    },
                );
                let mut dr = c_rect;
                DrawTextW(
                    mem_dc,
                    dw.as_ptr(),
                    (dw.len() - 1) as i32,
                    &mut dr,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );

                state.breadcrumbs.push(BreadcrumbItem {
                    path: crumb_path,
                    rect: c_rect,
                });

                bc_x += crumb_w + 4;
            }
        }
    } else {
        ShowWindow(path_edit_hwnd, SW_SHOW);
    }

    // Separator Line below Toolbar
    let sep_y = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + 4;
    SelectObject(mem_dc, state.pen_card_border);
    MoveToEx(mem_dc, main_left + 12, sep_y, ptr::null_mut());
    LineTo(mem_dc, width - CARD_MARGIN - 12, sep_y);

    if let Some(tab) = state.tabs.get(state.active_tab) {
        let is_searching = !tab.search_query.is_empty();
        let total_count = tab.items_count();

        let header_y = sep_y + 4;
        let main_w = width - CARD_MARGIN - main_left;
        let col_name_x = main_left + 18;
        let col_type_x = main_left + (main_w * 48 / 100);
        let col_size_x = main_left + (main_w * 64 / 100);
        let col_date_x = main_left + (main_w * 78 / 100);

        // 6. Column Headers (Only for Details and Compact views)
        if matches!(tab.view_mode, ViewMode::Details | ViewMode::Compact) {
            SelectObject(mem_dc, state.font_header);
            SetTextColor(mem_dc, CLR_SECONDARY_TEXT);

            let (sort_col, sort_asc) = (tab.sort_column, tab.sort_ascending);
            let arrow = if sort_asc { " ▲" } else { " ▼" };
            let name_header = if sort_col == SortColumn::Name {
                format!("Name{arrow}")
            } else {
                "Name".to_string()
            };
            let type_header = if sort_col == SortColumn::Type {
                format!("Type{arrow}")
            } else {
                "Type".to_string()
            };
            let size_header = if sort_col == SortColumn::Size {
                format!("Size{arrow}")
            } else {
                "Size".to_string()
            };
            let date_header = if sort_col == SortColumn::Date {
                format!("Date modified{arrow}")
            } else {
                "Date modified".to_string()
            };

            draw_header_text(
                mem_dc,
                col_name_x,
                header_y,
                col_type_x - col_name_x - 10,
                HEADER_HEIGHT,
                &name_header,
            );
            draw_header_text(
                mem_dc,
                col_type_x,
                header_y,
                col_size_x - col_type_x - 10,
                HEADER_HEIGHT,
                &type_header,
            );
            draw_header_text(
                mem_dc,
                col_size_x,
                header_y,
                col_date_x - col_size_x - 10,
                HEADER_HEIGHT,
                &size_header,
            );
            draw_header_text(
                mem_dc,
                col_date_x,
                header_y,
                (width - CARD_MARGIN) - col_date_x - 16,
                HEADER_HEIGHT,
                &date_header,
            );
        }

        // 7. File Content Area
        let header_h = match tab.view_mode {
            ViewMode::Details | ViewMode::Compact => HEADER_HEIGHT,
            ViewMode::Grid => 0,
        };
        let list_top = sep_y + 4 + header_h + 2;
        let list_bottom = height - CARD_MARGIN - STATUS_HEIGHT;

        SelectObject(mem_dc, state.font_normal);

        match tab.view_mode {
            ViewMode::Details => {
                let row_h = 34;
                let visible_rows = ((list_bottom - list_top) / row_h + 2) as usize;

                for i in 0..visible_rows {
                    let item_idx = tab.scroll_pos as usize + i;
                    if item_idx >= total_count {
                        break;
                    }

                    let row_y = list_top + (i as i32 * row_h);
                    if row_y >= list_bottom {
                        break;
                    }

                    if !is_searching && tab.group_by_date {
                        if let Some(DisplayRow::Header { group, count }) =
                            tab.display_rows.get(item_idx)
                        {
                            // 1. Clean top divider line separating groups
                            SelectObject(mem_dc, state.pen_card_border);
                            MoveToEx(mem_dc, main_left + 12, row_y + 2, ptr::null_mut());
                            LineTo(mem_dc, width - CARD_MARGIN - 18, row_y + 2);

                            // 2. Chevron indicator "▾" in ACCENT color
                            SelectObject(mem_dc, state.font_header);
                            SetTextColor(mem_dc, CLR_ACCENT);
                            let chevron_w = wide("▾");
                            let mut chev_rect = RECT {
                                left: col_name_x,
                                top: row_y + 4,
                                right: col_name_x + 14,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                chevron_w.as_ptr(),
                                (chevron_w.len() - 1) as i32,
                                &mut chev_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            // 3. Category label in bold ACCENT text
                            let title = group.label();
                            let w_title = wide(title);
                            let mut title_rect = RECT {
                                left: col_name_x + 16,
                                top: row_y + 4,
                                right: col_name_x + 250,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                w_title.as_ptr(),
                                (w_title.len() - 1) as i32,
                                &mut title_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            // 4. Count badge in soft secondary text
                            let mut title_size: POINT = zeroed();
                            GetTextExtentPoint32W(
                                mem_dc,
                                w_title.as_ptr(),
                                (w_title.len() - 1) as i32,
                                &mut title_size as *mut _ as _,
                            );
                            let count_str = format!("({})", count);
                            let w_count = wide(&count_str);
                            SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
                            SelectObject(mem_dc, state.font_small);
                            let mut count_rect = RECT {
                                left: col_name_x + 16 + title_size.x + 6,
                                top: row_y + 4,
                                right: col_name_x + 16 + title_size.x + 80,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                w_count.as_ptr(),
                                (w_count.len() - 1) as i32,
                                &mut count_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            SelectObject(mem_dc, state.font_normal);
                            continue;
                        }
                    }

                    SelectObject(mem_dc, state.font_normal);
                    let actual_entry_idx = if is_searching {
                        item_idx
                    } else if tab.group_by_date {
                        match tab.display_rows.get(item_idx) {
                            Some(DisplayRow::Item { entry_idx }) => *entry_idx,
                            _ => continue,
                        }
                    } else {
                        item_idx
                    };

                    let is_selected = tab.is_selected(actual_entry_idx);
                    let is_hovered = state.hovered_row == Some(item_idx);

                    let row_pill = RECT {
                        left: main_left + 8,
                        top: row_y + 1,
                        right: width - CARD_MARGIN - 18,
                        bottom: row_y + row_h - 1,
                    };

                    SelectObject(mem_dc, state.pen_null);
                    if is_selected {
                        SelectObject(mem_dc, state.brush_select);
                        RoundRect(
                            mem_dc,
                            row_pill.left,
                            row_pill.top,
                            row_pill.right,
                            row_pill.bottom,
                            PILL_RADIUS,
                            PILL_RADIUS,
                        );
                        let pill_accent = RECT {
                            left: row_pill.left + 3,
                            top: row_pill.top + 6,
                            right: row_pill.left + 6,
                            bottom: row_pill.bottom - 6,
                        };
                        FillRect(mem_dc, &pill_accent, state.brush_accent);
                    } else if is_hovered {
                        SelectObject(mem_dc, state.brush_hover);
                        RoundRect(
                            mem_dc,
                            row_pill.left,
                            row_pill.top,
                            row_pill.right,
                            row_pill.bottom,
                            PILL_RADIUS,
                            PILL_RADIUS,
                        );
                    }

                    let (name_w, is_dir, is_hidden, size, mtime) = if is_searching {
                        let res = &tab.search_results[item_idx];
                        (
                            &res.entry.name,
                            res.entry.is_dir,
                            res.entry.is_hidden,
                            res.entry.size,
                            res.entry.modified_time,
                        )
                    } else {
                        let ent = &tab.entries[actual_entry_idx];
                        (
                            &ent.name,
                            ent.is_dir,
                            ent.is_hidden,
                            ent.size,
                            ent.modified_time,
                        )
                    };

                    let (cat_name, cat_color) = classify_entry(name_w, is_dir);
                    let size_str = format_size(size, is_dir);
                    let date_str = format_filetime(mtime);

                    let item_indent = if !is_searching && tab.group_by_date {
                        12
                    } else {
                        0
                    };
                    if is_dir {
                        draw_vector_folder(
                            mem_dc,
                            col_name_x + 2 + item_indent,
                            row_y + 10,
                            cat_color,
                        );
                    } else {
                        draw_vector_file(
                            mem_dc,
                            col_name_x + 2 + item_indent,
                            row_y + 10,
                            cat_color,
                        );
                    }

                    SetTextColor(
                        mem_dc,
                        if is_hidden {
                            CLR_SECONDARY_TEXT
                        } else {
                            CLR_PRIMARY_TEXT
                        },
                    );
                    let col_name_rect = RECT {
                        left: col_name_x + 22 + item_indent,
                        top: row_y,
                        right: col_type_x - 10,
                        bottom: row_y + row_h,
                    };
                    draw_highlighted_text(
                        mem_dc,
                        name_w,
                        &tab.search_query,
                        &col_name_rect,
                        CLR_PRIMARY_TEXT,
                        CLR_ACCENT,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
                    let type_w = wide(cat_name);
                    let mut col_type_rect = RECT {
                        left: col_type_x,
                        top: row_y,
                        right: col_size_x - 10,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        type_w.as_ptr(),
                        (type_w.len() - 1) as i32,
                        &mut col_type_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    let size_w = wide(&size_str);
                    let mut col_size_rect = RECT {
                        left: col_size_x,
                        top: row_y,
                        right: col_date_x - 10,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        size_w.as_ptr(),
                        (size_w.len() - 1) as i32,
                        &mut col_size_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    let date_w = wide(&date_str);
                    let mut col_date_rect = RECT {
                        left: col_date_x,
                        top: row_y,
                        right: width - CARD_MARGIN - 24,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        date_w.as_ptr(),
                        (date_w.len() - 1) as i32,
                        &mut col_date_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
            }
            ViewMode::Compact => {
                let row_h = 26;
                let visible_rows = ((list_bottom - list_top) / row_h + 2) as usize;

                for i in 0..visible_rows {
                    let item_idx = tab.scroll_pos as usize + i;
                    if item_idx >= total_count {
                        break;
                    }

                    let row_y = list_top + (i as i32 * row_h);
                    if row_y >= list_bottom {
                        break;
                    }

                    if !is_searching && tab.group_by_date {
                        if let Some(DisplayRow::Header { group, count }) =
                            tab.display_rows.get(item_idx)
                        {
                            // 1. Clean top divider line separating groups
                            SelectObject(mem_dc, state.pen_card_border);
                            MoveToEx(mem_dc, main_left + 12, row_y + 1, ptr::null_mut());
                            LineTo(mem_dc, width - CARD_MARGIN - 18, row_y + 1);

                            // 2. Chevron indicator "▾" in ACCENT color
                            SelectObject(mem_dc, state.font_header);
                            SetTextColor(mem_dc, CLR_ACCENT);
                            let chevron_w = wide("▾");
                            let mut chev_rect = RECT {
                                left: col_name_x,
                                top: row_y + 2,
                                right: col_name_x + 14,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                chevron_w.as_ptr(),
                                (chevron_w.len() - 1) as i32,
                                &mut chev_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            // 3. Category label in bold ACCENT text
                            let title = group.label();
                            let w_title = wide(title);
                            let mut title_rect = RECT {
                                left: col_name_x + 16,
                                top: row_y + 2,
                                right: col_name_x + 250,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                w_title.as_ptr(),
                                (w_title.len() - 1) as i32,
                                &mut title_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            // 4. Count badge in soft secondary text
                            let mut title_size: POINT = zeroed();
                            GetTextExtentPoint32W(
                                mem_dc,
                                w_title.as_ptr(),
                                (w_title.len() - 1) as i32,
                                &mut title_size as *mut _ as _,
                            );
                            let count_str = format!("({})", count);
                            let w_count = wide(&count_str);
                            SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
                            SelectObject(mem_dc, state.font_small);
                            let mut count_rect = RECT {
                                left: col_name_x + 16 + title_size.x + 6,
                                top: row_y + 2,
                                right: col_name_x + 16 + title_size.x + 80,
                                bottom: row_y + row_h,
                            };
                            DrawTextW(
                                mem_dc,
                                w_count.as_ptr(),
                                (w_count.len() - 1) as i32,
                                &mut count_rect,
                                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                            );

                            SelectObject(mem_dc, state.font_normal);
                            continue;
                        }
                    }

                    SelectObject(mem_dc, state.font_normal);
                    let actual_entry_idx = if is_searching {
                        item_idx
                    } else if tab.group_by_date {
                        match tab.display_rows.get(item_idx) {
                            Some(DisplayRow::Item { entry_idx }) => *entry_idx,
                            _ => continue,
                        }
                    } else {
                        item_idx
                    };

                    let is_selected = tab.is_selected(actual_entry_idx);
                    let is_hovered = state.hovered_row == Some(item_idx);

                    let row_pill = RECT {
                        left: main_left + 8,
                        top: row_y + 1,
                        right: width - CARD_MARGIN - 18,
                        bottom: row_y + row_h - 1,
                    };

                    SelectObject(mem_dc, state.pen_null);
                    if is_selected {
                        SelectObject(mem_dc, state.brush_select);
                        RoundRect(
                            mem_dc,
                            row_pill.left,
                            row_pill.top,
                            row_pill.right,
                            row_pill.bottom,
                            PILL_RADIUS,
                            PILL_RADIUS,
                        );
                        let pill_accent = RECT {
                            left: row_pill.left + 3,
                            top: row_pill.top + 4,
                            right: row_pill.left + 6,
                            bottom: row_pill.bottom - 4,
                        };
                        FillRect(mem_dc, &pill_accent, state.brush_accent);
                    } else if is_hovered {
                        SelectObject(mem_dc, state.brush_hover);
                        RoundRect(
                            mem_dc,
                            row_pill.left,
                            row_pill.top,
                            row_pill.right,
                            row_pill.bottom,
                            PILL_RADIUS,
                            PILL_RADIUS,
                        );
                    }

                    let (name_w, is_dir, is_hidden, size, mtime) = if is_searching {
                        let res = &tab.search_results[item_idx];
                        (
                            &res.entry.name,
                            res.entry.is_dir,
                            res.entry.is_hidden,
                            res.entry.size,
                            res.entry.modified_time,
                        )
                    } else {
                        let ent = &tab.entries[actual_entry_idx];
                        (
                            &ent.name,
                            ent.is_dir,
                            ent.is_hidden,
                            ent.size,
                            ent.modified_time,
                        )
                    };

                    let (cat_name, cat_color) = classify_entry(name_w, is_dir);
                    let size_str = format_size(size, is_dir);
                    let date_str = format_filetime(mtime);

                    let item_indent = if !is_searching && tab.group_by_date {
                        12
                    } else {
                        0
                    };
                    if is_dir {
                        draw_vector_folder(
                            mem_dc,
                            col_name_x + 2 + item_indent,
                            row_y + 6,
                            cat_color,
                        );
                    } else {
                        draw_vector_file(
                            mem_dc,
                            col_name_x + 2 + item_indent,
                            row_y + 6,
                            cat_color,
                        );
                    }

                    SetTextColor(
                        mem_dc,
                        if is_hidden {
                            CLR_SECONDARY_TEXT
                        } else {
                            CLR_PRIMARY_TEXT
                        },
                    );
                    let col_name_rect = RECT {
                        left: col_name_x + 22 + item_indent,
                        top: row_y,
                        right: col_type_x - 10,
                        bottom: row_y + row_h,
                    };
                    draw_highlighted_text(
                        mem_dc,
                        name_w,
                        &tab.search_query,
                        &col_name_rect,
                        CLR_PRIMARY_TEXT,
                        CLR_ACCENT,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
                    let type_w = wide(cat_name);
                    let mut col_type_rect = RECT {
                        left: col_type_x,
                        top: row_y,
                        right: col_size_x - 10,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        type_w.as_ptr(),
                        (type_w.len() - 1) as i32,
                        &mut col_type_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    let size_w = wide(&size_str);
                    let mut col_size_rect = RECT {
                        left: col_size_x,
                        top: row_y,
                        right: col_date_x - 10,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        size_w.as_ptr(),
                        (size_w.len() - 1) as i32,
                        &mut col_size_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );

                    let date_w = wide(&date_str);
                    let mut col_date_rect = RECT {
                        left: col_date_x,
                        top: row_y,
                        right: width - CARD_MARGIN - 24,
                        bottom: row_y + row_h,
                    };
                    DrawTextW(
                        mem_dc,
                        date_w.as_ptr(),
                        (date_w.len() - 1) as i32,
                        &mut col_date_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
            }
            ViewMode::Grid => {
                let card_w = 108;
                let card_h = 92;
                let cell_w = 116;
                let cell_h = 100;
                let content_w = (width - CARD_MARGIN - main_left - 24).max(120);
                let cols = (content_w / cell_w).max(1) as usize;
                let visible_rows = ((list_bottom - list_top) / cell_h + 2) as usize;

                for r in 0..visible_rows {
                    let row_idx = tab.scroll_pos as usize + r;
                    for c in 0..cols {
                        let item_idx = row_idx * cols + c;
                        if item_idx >= total_count {
                            break;
                        }

                        let card_x = main_left + 14 + (c as i32 * cell_w);
                        let card_y = list_top + (r as i32 * cell_h);
                        if card_y + card_h > list_bottom {
                            break;
                        }

                        let is_selected = tab.is_selected(item_idx);
                        let is_hovered = state.hovered_row == Some(item_idx);

                        let card_rect = RECT {
                            left: card_x,
                            top: card_y,
                            right: card_x + card_w,
                            bottom: card_y + card_h,
                        };

                        SelectObject(mem_dc, state.pen_null);
                        if is_selected {
                            SelectObject(mem_dc, state.brush_select);
                            RoundRect(
                                mem_dc,
                                card_rect.left,
                                card_rect.top,
                                card_rect.right,
                                card_rect.bottom,
                                8,
                                8,
                            );
                            let accent_line = RECT {
                                left: card_rect.left + 8,
                                top: card_rect.bottom - 3,
                                right: card_rect.right - 8,
                                bottom: card_rect.bottom,
                            };
                            FillRect(mem_dc, &accent_line, state.brush_accent);
                        } else if is_hovered {
                            SelectObject(mem_dc, state.brush_hover);
                            RoundRect(
                                mem_dc,
                                card_rect.left,
                                card_rect.top,
                                card_rect.right,
                                card_rect.bottom,
                                8,
                                8,
                            );
                        }

                        let (name_w, is_dir, is_hidden) = if is_searching {
                            match tab.search_results.get(item_idx) {
                                Some(res) => {
                                    (&res.entry.name, res.entry.is_dir, res.entry.is_hidden)
                                }
                                None => break,
                            }
                        } else {
                            match tab.entries.get(item_idx) {
                                Some(ent) => (&ent.name, ent.is_dir, ent.is_hidden),
                                None => break,
                            }
                        };

                        let (_, cat_color) = classify_entry(name_w, is_dir);
                        let icon_x = card_x + (card_w - 48) / 2;
                        let icon_y = card_y + 8;
                        if is_dir {
                            draw_vector_folder_large(mem_dc, icon_x, icon_y, cat_color);
                        } else {
                            draw_vector_file_large(mem_dc, icon_x, icon_y, cat_color);
                        }

                        SetTextColor(
                            mem_dc,
                            if is_hidden {
                                CLR_SECONDARY_TEXT
                            } else {
                                CLR_PRIMARY_TEXT
                            },
                        );
                        let name_rect = RECT {
                            left: card_x + 4,
                            top: card_y + 54,
                            right: card_x + card_w - 4,
                            bottom: card_y + card_h - 4,
                        };
                        draw_highlighted_text(
                            mem_dc,
                            name_w,
                            &tab.search_query,
                            &name_rect,
                            CLR_PRIMARY_TEXT,
                            CLR_ACCENT,
                            DT_CENTER | DT_WORDBREAK | DT_END_ELLIPSIS | DT_NOPREFIX,
                        );
                    }
                }
            }
        }

        if total_count == 0 {
            SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
            let empty_msg = if is_searching {
                "No files match your filter."
            } else {
                "This folder is empty."
            };
            let msg_w = wide(empty_msg);
            let mut empty_rect = RECT {
                left: col_name_x + 8,
                top: list_top + 30,
                right: width - CARD_MARGIN - 24,
                bottom: list_top + 70,
            };
            DrawTextW(
                mem_dc,
                msg_w.as_ptr(),
                (msg_w.len() - 1) as i32,
                &mut empty_rect,
                DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
            );
        }

        // 7b. Modern On-App Scrollbar
        let sb_metrics = calc_scrollbar_metrics(width, height, tab);
        if sb_metrics.needed {
            SelectObject(mem_dc, state.pen_null);

            SelectObject(mem_dc, state.brush_scroll_track);
            RoundRect(
                mem_dc,
                sb_metrics.track_left,
                sb_metrics.track_top,
                sb_metrics.track_right,
                sb_metrics.track_bottom,
                3,
                3,
            );

            let thumb_brush = if state.scrollbar_dragging {
                state.brush_accent
            } else if state.scrollbar_hovered {
                state.brush_scroll_hover
            } else {
                state.brush_scroll_thumb
            };
            SelectObject(mem_dc, thumb_brush);
            RoundRect(
                mem_dc,
                sb_metrics.track_left,
                sb_metrics.thumb_top,
                sb_metrics.track_right,
                sb_metrics.thumb_bottom,
                3,
                3,
            );
        }

        // 8. Refined Status Bar
        let dir_count = tab.entries.iter().filter(|e| e.is_dir).count();
        let file_count = tab.entries.len().saturating_sub(dir_count);
        let total_size: u64 = tab.entries.iter().map(|e| e.size).sum();

        let sel_count = tab.selected.len();
        let (status_left, is_toast) = if let Some((ref toast, inst)) = state.status_toast {
            if inst.elapsed().as_secs_f32() < 4.0 {
                (toast.clone(), true)
            } else {
                let txt = if is_searching {
                    if tab
                        .search_handle
                        .as_ref()
                        .map_or(true, |h| h.cancel.load(Ordering::Relaxed))
                    {
                        format!("{} matches found", total_count)
                    } else {
                        format!("Searching... {} matches found", total_count)
                    }
                } else if sel_count > 1 {
                    let items = tab.selected_items();
                    let sel_size: u64 = items
                        .iter()
                        .filter_map(|it| std::fs::metadata(&it.path).ok().map(|m| m.len()))
                        .sum();
                    format!(
                        "{} items selected ({})",
                        sel_count,
                        format_size(sel_size, false)
                    )
                } else if sel_count == 1 {
                    if let Some(item) = tab.selected_item() {
                        if item.is_dir {
                            {
                                let folder_metrics = state
                                    .folder_size_cache
                                    .lock()
                                    .ok()
                                    .and_then(|c| c.get(&item.path).copied());
                                if let Some((sz, f_cnt, d_cnt)) = folder_metrics {
                                    format!("{} folders, {} files  •  Selected: {} ({}, {} files, {} folders)", dir_count, file_count, item.name, format_size(sz, false), f_cnt, d_cnt)
                                } else {
                                    format!("{} folders, {} files  •  Selected: {} (Folder - Space to calculate)", dir_count, file_count, item.name)
                                }
                            }
                        } else {
                            let sz = std::fs::metadata(&item.path).map_or(0, |m| m.len());
                            format!(
                                "{} folders, {} files  •  Selected: {} ({})",
                                dir_count,
                                file_count,
                                item.name,
                                format_size(sz, false)
                            )
                        }
                    } else {
                        format!(
                            "{} folders, {} files ({})",
                            dir_count,
                            file_count,
                            format_size(total_size, false)
                        )
                    }
                } else {
                    format!(
                        "{} folders, {} files ({})",
                        dir_count,
                        file_count,
                        format_size(total_size, false)
                    )
                };
                (txt, false)
            }
        } else {
            let txt = if is_searching {
                if tab
                    .search_handle
                    .as_ref()
                    .map_or(true, |h| h.cancel.load(Ordering::Relaxed))
                {
                    format!("{} matches found", total_count)
                } else {
                    format!("Searching... {} matches found", total_count)
                }
            } else if sel_count > 1 {
                let items = tab.selected_items();
                let sel_size: u64 = items
                    .iter()
                    .filter_map(|it| std::fs::metadata(&it.path).ok().map(|m| m.len()))
                    .sum();
                format!(
                    "{} items selected ({})",
                    sel_count,
                    format_size(sel_size, false)
                )
            } else if sel_count == 1 {
                if let Some(item) = tab.selected_item() {
                    if item.is_dir {
                        {
                            let folder_metrics = state
                                .folder_size_cache
                                .lock()
                                .ok()
                                .and_then(|c| c.get(&item.path).copied());
                            if let Some((sz, f_cnt, d_cnt)) = folder_metrics {
                                format!("{} folders, {} files  •  Selected: {} ({}, {} files, {} folders)", dir_count, file_count, item.name, format_size(sz, false), f_cnt, d_cnt)
                            } else {
                                format!("{} folders, {} files  •  Selected: {} (Folder - Space to calculate)", dir_count, file_count, item.name)
                            }
                        }
                    } else {
                        let sz = std::fs::metadata(&item.path).map_or(0, |m| m.len());
                        format!(
                            "{} folders, {} files  •  Selected: {} ({})",
                            dir_count,
                            file_count,
                            item.name,
                            format_size(sz, false)
                        )
                    }
                } else {
                    format!(
                        "{} folders, {} files ({})",
                        dir_count,
                        file_count,
                        format_size(total_size, false)
                    )
                }
            } else {
                format!(
                    "{} folders, {} files ({})",
                    dir_count,
                    file_count,
                    format_size(total_size, false)
                )
            };
            (txt, false)
        };

        let status_w = wide(&status_left);
        if is_toast {
            SetTextColor(mem_dc, CLR_ACCENT);
        } else {
            SetTextColor(mem_dc, CLR_SECONDARY_TEXT);
        }
        let mut status_rect = RECT {
            left: main_left + 16,
            top: height - CARD_MARGIN - STATUS_HEIGHT,
            right: width - CARD_MARGIN - 260,
            bottom: height - CARD_MARGIN,
        };
        DrawTextW(
            mem_dc,
            status_w.as_ptr(),
            (status_w.len() - 1) as i32,
            &mut status_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );

        let tips = wide("● Auto-synced  •  Space: Preview  •  Ctrl+L: Path");
        let mut tips_rect = RECT {
            left: width - CARD_MARGIN - 340,
            top: height - CARD_MARGIN - STATUS_HEIGHT,
            right: width - CARD_MARGIN - 16,
            bottom: height - CARD_MARGIN,
        };
        DrawTextW(
            mem_dc,
            tips.as_ptr(),
            (tips.len() - 1) as i32,
            &mut tips_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
        );
    }

    BitBlt(target_dc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

    SelectObject(mem_dc, old_bmp);
    DeleteObject(mem_bmp);
    DeleteDC(mem_dc);
    EndPaint(hwnd, &ps);
}
unsafe fn draw_header_text(dc: HDC, x: i32, y: i32, w: i32, h: i32, text: &str) {
    let mut r = RECT {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    };
    let w_text = wide(text);
    DrawTextW(
        dc,
        w_text.as_ptr(),
        (w_text.len() - 1) as i32,
        &mut r,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
    );
}

fn parse_cli_target() -> (PathBuf, Option<String>) {
    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 {
        let default_dir = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\"));
        return (default_dir, None);
    }

    let mut raw = args[1].trim().to_string();
    let mut selected_file = None;

    if raw.to_lowercase().starts_with("/select,") {
        let rest = raw[8..].trim().trim_matches('"');
        let p = PathBuf::from(rest);
        if let Some(parent) = p.parent() {
            if let Some(file_name) = p.file_name() {
                selected_file = Some(file_name.to_string_lossy().to_string());
            }
            raw = parent.to_string_lossy().to_string();
        }
    }

    let cleaned = raw.trim_matches('"').trim();
    let p = PathBuf::from(cleaned);

    if p.is_dir() {
        (p, selected_file)
    } else if p.is_file() {
        let parent = p
            .parent()
            .unwrap_or_else(|| Path::new("C:\\"))
            .to_path_buf();
        let fname = p.file_name().map(|n| n.to_string_lossy().to_string());
        (parent, fname)
    } else if p.exists() {
        (p, selected_file)
    } else {
        let default_dir = std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\"));
        (default_dir, None)
    }
}

fn set_reg_value(hkey: HKEY, subkey: &str, value_name: Option<&str>, val: &str) -> bool {
    unsafe {
        let subkey_w = wide(subkey);
        let mut h_sub = ptr::null_mut();
        let res = RegCreateKeyW(hkey, subkey_w.as_ptr(), &mut h_sub);
        if res != 0 || h_sub.is_null() {
            return false;
        }

        let val_w = wide(val);
        let val_bytes = (val_w.len() * 2) as u32;

        let name_w = value_name.map(wide);
        let name_ptr = name_w.as_ref().map_or(ptr::null(), |v| v.as_ptr());

        let set_res = RegSetValueExW(
            h_sub,
            name_ptr,
            0,
            REG_SZ,
            val_w.as_ptr() as *const u8,
            val_bytes,
        );
        RegCloseKey(h_sub);
        set_res == 0
    }
}

fn ensure_shortcuts() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let start_dir = PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs");
            let lnk = start_dir.join("Rovik.lnk");
            let taskbar_lnk = PathBuf::from(&appdata)
                .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar\rovik.lnk");

            let exe_str = exe.to_string_lossy().replace('\'', "''");
            let lnk_str = lnk.to_string_lossy().replace('\'', "''");
            let tb_str = taskbar_lnk.to_string_lossy().replace('\'', "''");

            let script = format!(
                "$sh = New-Object -ComObject WScript.Shell;\
                 if (-not (Test-Path '{lnk}')) {{ $s = $sh.CreateShortcut('{lnk}'); $s.TargetPath = '{exe}'; $s.IconLocation = '{exe},0'; $s.Save() }};\
                 if (Test-Path '{tb}') {{ $s = $sh.CreateShortcut('{tb}'); $s.TargetPath = '{exe}'; $s.IconLocation = '{exe},0'; $s.Save() }}",
                lnk = lnk_str,
                exe = exe_str,
                tb = tb_str
            );

            let _ = std::process::Command::new("powershell")
                .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
                .creation_flags(0x08000000)
                .status();
        }
    }
}

pub fn register_as_default_explorer() -> bool {
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let exe_str = current_exe.to_string_lossy().to_string();
    let cmd_str = format!("\"{}\" \"%1\"", exe_str);
    let win_e_cmd = format!("\"{}\"", exe_str);

    let mut success = true;

    // 1. Folder
    success &= set_reg_value(
        HKEY_CURRENT_USER,
        r"Software\Classes\Folder\shell\open\command",
        None,
        &cmd_str,
    );
    success &= set_reg_value(
        HKEY_CURRENT_USER,
        r"Software\Classes\Folder\shell\open\command",
        Some("DelegateExecute"),
        "",
    );

    // 2. Directory
    success &= set_reg_value(
        HKEY_CURRENT_USER,
        r"Software\Classes\Directory\shell\open\command",
        None,
        &cmd_str,
    );

    // 3. Drive
    success &= set_reg_value(
        HKEY_CURRENT_USER,
        r"Software\Classes\Drive\shell\open\command",
        None,
        &cmd_str,
    );

    // 4. Win + E Hotkey Redirection (CLSID {52205fd8-5dfb-447d-801a-d0b52f2e83e1})
    let clsid_key = r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command";
    success &= set_reg_value(HKEY_CURRENT_USER, clsid_key, None, &win_e_cmd);
    success &= set_reg_value(HKEY_CURRENT_USER, clsid_key, Some("DelegateExecute"), "");

    ensure_shortcuts();
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            ptr::null(),
            ptr::null(),
        );
        let title = wide("Rovik - Default File Explorer");
        let msg = if success {
            wide("Rovik has been successfully set as your default Windows File Explorer!\n\n• Double-clicking folders opens in Rovik\n• Opening drives opens in Rovik\n• Win + E shortcut opens Rovik\n\nTo restore default Windows Explorer at any time, run Rovik with --unregister.")
        } else {
            wide("Failed to update some registry keys. Please ensure you have write access to your user registry.")
        };
        MessageBoxW(
            ptr::null_mut(),
            msg.as_ptr(),
            title.as_ptr(),
            MB_OK
                | if success {
                    MB_ICONINFORMATION
                } else {
                    MB_ICONWARNING
                },
        );
    }

    success
}

pub fn unregister_default_explorer() -> bool {
    unsafe {
        let _ = RegDeleteTreeW(
            HKEY_CURRENT_USER,
            wide(r"Software\Classes\Folder\shell\open").as_ptr(),
        );
        let _ = RegDeleteTreeW(
            HKEY_CURRENT_USER,
            wide(r"Software\Classes\Directory\shell\open").as_ptr(),
        );
        let _ = RegDeleteTreeW(
            HKEY_CURRENT_USER,
            wide(r"Software\Classes\Drive\shell\open").as_ptr(),
        );
        let _ = RegDeleteTreeW(
            HKEY_CURRENT_USER,
            wide(r"Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}").as_ptr(),
        );
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            ptr::null(),
            ptr::null(),
        );

        let title = wide("Rovik - Default File Explorer");
        let msg = wide("Windows default File Explorer has been restored.\n\nAll folder and drive associations have been reset to Windows defaults.");
        MessageBoxW(
            ptr::null_mut(),
            msg.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
    true
}

pub fn run() {
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(wide("Rovik.FileExplorer.1.0").as_ptr());
        // Set DPI awareness for proper high-DPI scaling
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        drag_drop::OleInitialize(ptr::null());
    }
    ensure_shortcuts();

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        let flag = args[1].trim();
        if flag.eq_ignore_ascii_case("--register") {
            register_as_default_explorer();
            return;
        } else if flag.eq_ignore_ascii_case("--unregister") {
            unregister_default_explorer();
            return;
        }
    }
    unsafe {
        let instance: HINSTANCE = GetModuleHandleW(ptr::null());
        let class_name = wide("RovikMainWindow");

        let hicon = LoadIconW(instance, 1 as _);
        let sm_cx = GetSystemMetrics(SM_CXSMICON);
        let sm_cy = GetSystemMetrics(SM_CYSMICON);
        let hicon_sm = LoadImageW(
            instance,
            1 as _,
            IMAGE_ICON,
            if sm_cx > 0 { sm_cx } else { 16 },
            if sm_cy > 0 { sm_cy } else { 16 },
            0,
        ) as HICON;

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: if !hicon.is_null() {
                hicon
            } else {
                ptr::null_mut()
            },
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: GetStockObject(NULL_BRUSH as i32) as _,
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };

        RegisterClassW(&wc);

        let font_family = select_font_family();

        let font_normal = CreateFontW(
            -15,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_family.as_ptr(),
        );

        let font_header = CreateFontW(
            -15,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_family.as_ptr(),
        );

        let font_small = CreateFontW(
            -11,
            0,
            0,
            0,
            FW_SEMIBOLD as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32 | FF_DONTCARE as u32,
            font_family.as_ptr(),
        );

        let loaded_settings = Settings::load();
        FOLDERS_FIRST.store(loaded_settings.folders_first, Ordering::Relaxed);

        let (cli_dir, select_file) = parse_cli_target();
        let initial_dir = if std::env::args().len() > 1 {
            cli_dir
        } else {
            match loaded_settings.startup_mode {
                StartupMode::Home => cli_dir,
                StartupMode::LastSession => {
                    if let Some(first) = loaded_settings.last_session.first() {
                        let p = PathBuf::from(first);
                        if p.is_dir() {
                            p
                        } else {
                            cli_dir
                        }
                    } else {
                        cli_dir
                    }
                }
                StartupMode::Custom => {
                    if !loaded_settings.startup_folder.is_empty() {
                        let p = PathBuf::from(&loaded_settings.startup_folder);
                        if p.is_dir() {
                            p
                        } else {
                            cli_dir
                        }
                    } else {
                        cli_dir
                    }
                }
            }
        };

        let mut initial_tabs = Vec::new();
        if std::env::args().len() <= 1
            && loaded_settings.startup_mode == StartupMode::LastSession
            && !loaded_settings.last_session.is_empty()
        {
            for path_str in &loaded_settings.last_session {
                let p = PathBuf::from(path_str);
                if p.is_dir() {
                    initial_tabs.push(Tab::new(
                        p,
                        loaded_settings.show_hidden,
                        loaded_settings.view_mode,
                    ));
                }
            }
        }
        if initial_tabs.is_empty() {
            let mut tab = Tab::new(
                initial_dir,
                loaded_settings.show_hidden,
                loaded_settings.view_mode,
            );
            if let Some(ref target_name) = select_file {
                let target_lower = target_name.to_lowercase();
                if let Some(idx) = tab.entries.iter().position(|e| {
                    OsString::from_wide(&e.name)
                        .to_string_lossy()
                        .to_lowercase()
                        == target_lower
                }) {
                    tab.select_single(idx);
                    let main_top = CARD_MARGIN + TAB_BAR_HEIGHT + NAV_BAR_HEIGHT + HEADER_HEIGHT;
                    let main_bottom = 740 - CARD_MARGIN - STATUS_HEIGHT;
                    let page_items = ((main_bottom - main_top) / ROW_HEIGHT).max(1) as usize;
                    if idx >= page_items {
                        tab.scroll_pos = (idx - page_items / 2) as i32;
                    }
                }
            }
            initial_tabs.push(tab);
        }

        let sidebar_entries = build_sidebar_entries(&loaded_settings.pinned_folders);

        let mut state = Box::new(State {
            tabs: initial_tabs,
            active_tab: 0,
            sidebar_entries,
            sidebar_hovered: None,
            tab_hovered: None,
            tab_close_hovered: None,
            tab_plus_hovered: false,
            btn_hovered: None,
            hovered_row: None,
            show_hidden: loaded_settings.show_hidden,
            tracking_mouse: false,
            clipboard: None,
            breadcrumbs: Vec::new(),
            hovered_breadcrumb: None,
            is_path_editing: false,
            dir_watcher: None,
            scrollbar_hovered: false,
            scrollbar_dragging: false,
            scrollbar_drag_offset_y: 0,
            drag_start: None,
            drag_candidate: None,
            drag_deferred_select: false,
            settings: loaded_settings,
            settings_btn_hovered: false,
            dual_pane: false,
            dual_active: 0,
            left_tab: 0,
            right_tab: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            status_toast: None,
            cached_open_with: Vec::new(),
            cached_send_to: Vec::new(),
            sidebar_layout_cache: None,
            sidebar_layout_height: 0,
            scrollbar_cache: None,
            scrollbar_cache_rect: (0, 0),
            breadcrumbs_cached_path: None,
            folder_size_cache: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),

            brush_outer: CreateSolidBrush(CLR_OUTER_BG),
            brush_sidebar: CreateSolidBrush(CLR_SIDEBAR_BG),
            brush_main_card: CreateSolidBrush(CLR_MAIN_CARD_BG),
            brush_card_border: CreateSolidBrush(CLR_CARD_BORDER),
            brush_select: CreateSolidBrush(CLR_ROW_SELECT),
            brush_hover: CreateSolidBrush(CLR_ROW_HOVER),
            brush_btn: CreateSolidBrush(CLR_BTN_BG),
            brush_accent: CreateSolidBrush(CLR_ACCENT),
            brush_tab_inactive: CreateSolidBrush(CLR_TAB_INACTIVE),
            brush_input: CreateSolidBrush(CLR_INPUT_BG),
            brush_scroll_track: CreateSolidBrush(CLR_SCROLL_TRACK),
            brush_scroll_thumb: CreateSolidBrush(CLR_SCROLL_THUMB),
            brush_scroll_hover: CreateSolidBrush(CLR_SCROLL_HOVER),
            pen_null: CreatePen(PS_NULL as i32, 0, 0),
            pen_card_border: CreatePen(PS_SOLID as i32, 1, CLR_CARD_BORDER),
            pen_accent: CreatePen(PS_SOLID as i32, 1, CLR_ACCENT),
            font_normal,
            font_header,
            font_small,
            app_icon: if !hicon_sm.is_null() { hicon_sm } else { hicon },
        });

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("Rovik").as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1180,
            740,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        );

        if hwnd.is_null() {
            return;
        }

        // Enable Drag & Drop
        DragAcceptFiles(hwnd, 1);

        if !hicon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, hicon as LPARAM);
        }
        if !hicon_sm.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, hicon_sm as LPARAM);
        }

        // Apply Windows 11 Immersive Dark Mode, Mica Backdrop, and Rounded Window Corners
        apply_modern_window_effects(hwnd);

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, &mut *state as *mut State as isize);
        ensure_dir_watcher(hwnd, &mut state);

        create_edit(hwnd, PATH_ID, state.font_normal, "Path...");
        create_edit(hwnd, SEARCH_ID, state.font_normal, "Filter...");

        SetTimer(hwnd, 1, 200, None); // 200ms = 5Hz, reduces CPU wakeups when idle

        // Comprehensive Power-user Accelerators
        let mut accels = [
            // Focus & Search
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'L' as u16,
                cmd: CMD_FOCUS_PATH as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FALT) as u8,
                key: b'D' as u16,
                cmd: CMD_FOCUS_PATH as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'F' as u16,
                cmd: CMD_FOCUS_SEARCH as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_F3,
                cmd: CMD_FOCUS_SEARCH as u16,
            },
            // Tabs
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'T' as u16,
                cmd: CMD_OPEN_NEW_TAB as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'W' as u16,
                cmd: CMD_TAB_CLOSE as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: VK_TAB,
                cmd: CMD_NEXT_TAB as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: VK_TAB,
                cmd: CMD_PREV_TAB as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'1' as u16,
                cmd: CMD_TAB_1 as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'2' as u16,
                cmd: CMD_TAB_2 as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'3' as u16,
                cmd: CMD_TAB_3 as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'4' as u16,
                cmd: CMD_TAB_4 as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'5' as u16,
                cmd: CMD_TAB_5 as u16,
            },
            // File Operations
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'Z' as u16,
                cmd: CMD_UNDO as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'Y' as u16,
                cmd: CMD_REDO as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'C' as u16,
                cmd: CMD_COPY as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'X' as u16,
                cmd: CMD_CUT as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'V' as u16,
                cmd: CMD_PASTE as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'D' as u16,
                cmd: CMD_DUPLICATE as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_F2,
                cmd: CMD_RENAME as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_DELETE,
                cmd: CMD_DELETE as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'N' as u16,
                cmd: CMD_NEW_FOLDER as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'C' as u16,
                cmd: CMD_COPY_PATH as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'U' as u16,
                cmd: CMD_ZIP_COMPRESS as u16,
            },
            // Tools & Developer
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'Z' as u16,
                cmd: CMD_OPEN_ZED as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'K' as u16,
                cmd: CMD_OPEN_CODIUM as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'T' as u16,
                cmd: CMD_TERMINAL as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'E' as u16,
                cmd: CMD_SHOW_IN_EXPLORER as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'H' as u16,
                cmd: CMD_CHECKSUM_SHA256 as u16,
            },
            // View & Refresh
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_F5,
                cmd: CMD_REFRESH as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'R' as u16,
                cmd: CMD_REFRESH as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'H' as u16,
                cmd: CMD_TOGGLE_HIDDEN as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FALT) as u8,
                key: VK_RETURN,
                cmd: CMD_PROPERTIES as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_F1,
                cmd: CMD_HELP_SHORTCUTS as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FSHIFT) as u8,
                key: VK_DELETE,
                cmd: CMD_DELETE_PERMANENT as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: VK_OEM_COMMA,
                cmd: CMD_SETTINGS as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL) as u8,
                key: b'A' as u16,
                cmd: CMD_SELECT_ALL as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'1' as u16,
                cmd: CMD_VIEW_DETAILS as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'2' as u16,
                cmd: CMD_VIEW_COMPACT as u16,
            },
            ACCEL {
                fVirt: (FVIRTKEY | FCONTROL | FSHIFT) as u8,
                key: b'3' as u16,
                cmd: CMD_VIEW_GRID as u16,
            },
            ACCEL {
                fVirt: FVIRTKEY as u8,
                key: VK_F10,
                cmd: CMD_TOGGLE_DUAL_PANE as u16,
            },
        ];
        let haccel = CreateAcceleratorTableW(accels.as_mut_ptr(), accels.len() as i32);

        update_controls_for_active_tab(hwnd, &state);
        update_scroll_info(hwnd, &mut state);

        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);

        let mut msg = zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) > 0 {
            if TranslateAcceleratorW(hwnd, haccel, &msg) == 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        DestroyAcceleratorTable(haccel);

        DeleteObject(state.brush_outer);
        DeleteObject(state.brush_sidebar);
        DeleteObject(state.brush_main_card);
        DeleteObject(state.brush_card_border);
        DeleteObject(state.brush_select);
        DeleteObject(state.brush_hover);
        DeleteObject(state.brush_btn);
        DeleteObject(state.brush_accent);
        DeleteObject(state.brush_tab_inactive);
        DeleteObject(state.brush_input);
        DeleteObject(state.pen_null);
        DeleteObject(state.pen_card_border);
        DeleteObject(state.pen_accent);
        DeleteObject(state.font_normal);
        DeleteObject(state.font_header);
        DeleteObject(state.font_small);
        drag_drop::OleUninitialize();
    }
}

#[cfg(test)]
mod date_tests {
    use super::*;

    #[test]
    fn test_classify_filetime_intervals() {
        let cur_days = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            + TIMEZONE_OFFSET_SECS)
            / 86_400;
        let days_to_ft = |days: i64| -> u64 {
            let local_sec = days * 86_400;
            let utc_sec = (local_sec - TIMEZONE_OFFSET_SECS) as u64;
            (utc_sec + 11_644_473_600) * 10_000_000
        };

        // Today
        let ft_today = days_to_ft(cur_days);
        assert_eq!(classify_filetime(ft_today, cur_days), DateGroup::Today);

        // Yesterday
        let ft_yesterday = days_to_ft(cur_days - 1);
        assert_eq!(
            classify_filetime(ft_yesterday, cur_days),
            DateGroup::Yesterday
        );

        // 400 days ago -> Older
        let ft_older = days_to_ft(cur_days - 400);
        assert_eq!(classify_filetime(ft_older, cur_days), DateGroup::Older);
    }
}
