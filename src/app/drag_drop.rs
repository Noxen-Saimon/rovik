#![allow(non_snake_case, dead_code)]

use std::{
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    ptr,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
};
use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    System::{
        DataExchange::RegisterClipboardFormatW,
        Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::Input::KeyboardAndMouse::{GetKeyState, VK_SHIFT},
};

unsafe extern "system" {
    fn GlobalFree(hmem: *mut c_void) -> *mut c_void;
}

// COM GUID definition
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

pub const IID_IUNKNOWN: GUID = GUID {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

pub const IID_IDATAOBJECT: GUID = GUID {
    data1: 0x0000010e,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

pub const IID_IDROPSOURCE: GUID = GUID {
    data1: 0x00000121,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

pub const IID_IENUMFORMATETC: GUID = GUID {
    data1: 0x00000103,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

// HRESULT Constants
pub const S_OK: i32 = 0;
pub const S_FALSE: i32 = 1;
pub const E_NOTIMPL: i32 = 0x80004001u32 as i32;
pub const E_NOINTERFACE: i32 = 0x80004002u32 as i32;
pub const E_POINTER: i32 = 0x80004003u32 as i32;
pub const E_INVALIDARG: i32 = 0x80070057u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8007000Eu32 as i32;
pub const DV_E_FORMATETC: i32 = 0x80040064u32 as i32;
pub const OLE_E_ADVISENOTSUPPORTED: i32 = 0x80040003u32 as i32;

pub const DRAGDROP_S_DROP: i32 = 0x00040100;
pub const DRAGDROP_S_CANCEL: i32 = 0x00040101;
pub const DRAGDROP_S_USEDEFAULTCURSORS: i32 = 0x00040102;

// OLE & DragDrop Constants
pub const CF_HDROP: u16 = 15;
pub const TYMED_HGLOBAL: u32 = 1;
pub const DVASPECT_CONTENT: u32 = 1;
pub const DATADIR_GET: u32 = 1;

pub const DROPEFFECT_NONE: u32 = 0;
pub const DROPEFFECT_COPY: u32 = 1;
pub const DROPEFFECT_MOVE: u32 = 2;

pub const MK_LBUTTON: u32 = 0x0001;
pub const MK_SHIFT: u32 = 0x0004;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FORMATETC {
    pub cfFormat: u16,
    pub ptd: *mut c_void,
    pub dwAspect: u32,
    pub lindex: i32,
    pub tymed: u32,
}

#[repr(C)]
pub struct STGMEDIUM {
    pub tymed: u32,
    pub hGlobal: *mut c_void,
    pub pUnkForRelease: *mut c_void,
}

#[repr(C)]
pub struct DROPFILES {
    pub pFiles: u32,
    pub pt: POINT,
    pub fNC: i32,
    pub fWide: i32,
}

#[link(name = "ole32")]
unsafe extern "system" {
    pub fn OleInitialize(pvreserved: *const c_void) -> i32;
    pub fn OleUninitialize();
    pub fn DoDragDrop(
        pdataobj: *mut c_void,
        pdropsrc: *mut c_void,
        dwokeffects: u32,
        pdweffect: *mut u32,
    ) -> i32;
}

#[inline]
fn guid_eq(a: *const GUID, b: &GUID) -> bool {
    if a.is_null() {
        return false;
    }
    unsafe {
        (*a).data1 == b.data1
            && (*a).data2 == b.data2
            && (*a).data3 == b.data3
            && (*a).data4 == b.data4
    }
}

// ---------------------------------------------------------------------------
// IEnumFORMATETC implementation
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct IEnumFORMATETCVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> i32,
    pub Skip: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    pub Reset: unsafe extern "system" fn(*mut c_void) -> i32,
    pub Clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
}

static ENUM_FORMATETC_VTBL: IEnumFORMATETCVtbl = IEnumFORMATETCVtbl {
    QueryInterface: enum_formatetc_query_interface,
    AddRef: enum_formatetc_add_ref,
    Release: enum_formatetc_release,
    Next: enum_formatetc_next,
    Skip: enum_formatetc_skip,
    Reset: enum_formatetc_reset,
    Clone: enum_formatetc_clone,
};

#[repr(C)]
struct RovikEnumFormatEtc {
    lpVtbl: *const IEnumFORMATETCVtbl,
    ref_count: AtomicU32,
    formats: Vec<FORMATETC>,
    index: usize,
}

unsafe extern "system" fn enum_formatetc_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> i32 {
    if ppv.is_null() {
        return E_POINTER;
    }
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_IENUMFORMATETC) {
        let obj = &*(this as *mut RovikEnumFormatEtc);
        obj.ref_count.fetch_add(1, Ordering::SeqCst);
        *ppv = this;
        S_OK
    } else {
        *ppv = ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn enum_formatetc_add_ref(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikEnumFormatEtc);
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn enum_formatetc_release(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikEnumFormatEtc);
    let old = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if old == 1 {
        drop(Box::from_raw(this as *mut RovikEnumFormatEtc));
        0
    } else {
        old - 1
    }
}

unsafe extern "system" fn enum_formatetc_next(
    this: *mut c_void,
    celt: u32,
    rgelt: *mut FORMATETC,
    pcelt_fetched: *mut u32,
) -> i32 {
    if rgelt.is_null() || (celt > 1 && pcelt_fetched.is_null()) {
        return E_INVALIDARG;
    }
    let obj = &mut *(this as *mut RovikEnumFormatEtc);
    let mut fetched = 0u32;
    while obj.index < obj.formats.len() && fetched < celt {
        *rgelt.add(fetched as usize) = obj.formats[obj.index];
        obj.index += 1;
        fetched += 1;
    }
    if !pcelt_fetched.is_null() {
        *pcelt_fetched = fetched;
    }
    if fetched == celt {
        S_OK
    } else {
        S_FALSE
    }
}

unsafe extern "system" fn enum_formatetc_skip(this: *mut c_void, celt: u32) -> i32 {
    let obj = &mut *(this as *mut RovikEnumFormatEtc);
    obj.index = (obj.index + celt as usize).min(obj.formats.len());
    S_OK
}

unsafe extern "system" fn enum_formatetc_reset(this: *mut c_void) -> i32 {
    let obj = &mut *(this as *mut RovikEnumFormatEtc);
    obj.index = 0;
    S_OK
}

unsafe extern "system" fn enum_formatetc_clone(this: *mut c_void, ppenum: *mut *mut c_void) -> i32 {
    if ppenum.is_null() {
        return E_POINTER;
    }
    let obj = &*(this as *mut RovikEnumFormatEtc);
    let clone = Box::new(RovikEnumFormatEtc {
        lpVtbl: &ENUM_FORMATETC_VTBL,
        ref_count: AtomicU32::new(1),
        formats: obj.formats.clone(),
        index: obj.index,
    });
    *ppenum = Box::into_raw(clone) as *mut c_void;
    S_OK
}

// ---------------------------------------------------------------------------
// IDropSource implementation
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct IDropSourceVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub QueryContinueDrag: unsafe extern "system" fn(*mut c_void, i32, u32) -> i32,
    pub GiveFeedback: unsafe extern "system" fn(*mut c_void, u32) -> i32,
}

static DROP_SOURCE_VTBL: IDropSourceVtbl = IDropSourceVtbl {
    QueryInterface: drop_source_query_interface,
    AddRef: drop_source_add_ref,
    Release: drop_source_release,
    QueryContinueDrag: drop_source_query_continue_drag,
    GiveFeedback: drop_source_give_feedback,
};

#[repr(C)]
pub struct RovikDropSource {
    lpVtbl: *const IDropSourceVtbl,
    ref_count: AtomicU32,
    pub shift_pressed_at_drop: AtomicBool,
}

unsafe extern "system" fn drop_source_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> i32 {
    if ppv.is_null() {
        return E_POINTER;
    }
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_IDROPSOURCE) {
        let obj = &*(this as *mut RovikDropSource);
        obj.ref_count.fetch_add(1, Ordering::SeqCst);
        *ppv = this;
        S_OK
    } else {
        *ppv = ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn drop_source_add_ref(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikDropSource);
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn drop_source_release(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikDropSource);
    let old = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if old == 1 {
        drop(Box::from_raw(this as *mut RovikDropSource));
        0
    } else {
        old - 1
    }
}

unsafe extern "system" fn drop_source_query_continue_drag(
    this: *mut c_void,
    escape_pressed: i32,
    key_state: u32,
) -> i32 {
    if escape_pressed != 0 {
        return DRAGDROP_S_CANCEL;
    }
    if (key_state & MK_LBUTTON) == 0 {
        // User released the left mouse button -> commit DROP!
        let obj = &*(this as *mut RovikDropSource);
        let shift = (key_state & MK_SHIFT) != 0;
        obj.shift_pressed_at_drop.store(shift, Ordering::SeqCst);
        return DRAGDROP_S_DROP;
    }
    S_OK
}

unsafe extern "system" fn drop_source_give_feedback(_this: *mut c_void, _effect: u32) -> i32 {
    // Zero-bloat standard Windows drag cursor
    DRAGDROP_S_USEDEFAULTCURSORS
}

// ---------------------------------------------------------------------------
// IDataObject implementation
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct IDataObjectVtbl {
    pub QueryInterface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32,
    pub AddRef: unsafe extern "system" fn(*mut c_void) -> u32,
    pub Release: unsafe extern "system" fn(*mut c_void) -> u32,
    pub GetData: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> i32,
    pub GetDataHere: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> i32,
    pub QueryGetData: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> i32,
    pub GetCanonicalFormatEtc: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> i32,
    pub SetData: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *const STGMEDIUM, i32) -> i32,
    pub EnumFormatEtc: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32,
    pub DAdvise: unsafe extern "system" fn(*mut c_void, *const FORMATETC, u32, *mut c_void, *mut u32) -> i32,
    pub DUnadvise: unsafe extern "system" fn(*mut c_void, u32) -> i32,
    pub EnumDAdvise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> i32,
}

static DATA_OBJECT_VTBL: IDataObjectVtbl = IDataObjectVtbl {
    QueryInterface: data_object_query_interface,
    AddRef: data_object_add_ref,
    Release: data_object_release,
    GetData: data_object_get_data,
    GetDataHere: data_object_get_data_here,
    QueryGetData: data_object_query_get_data,
    GetCanonicalFormatEtc: data_object_get_canonical_format_etc,
    SetData: data_object_set_data,
    EnumFormatEtc: data_object_enum_format_etc,
    DAdvise: data_object_d_advise,
    DUnadvise: data_object_d_unadvise,
    EnumDAdvise: data_object_enum_d_advise,
};

#[repr(C)]
pub struct RovikDataObject {
    lpVtbl: *const IDataObjectVtbl,
    ref_count: AtomicU32,
    paths: Vec<PathBuf>,
    format_drop_effect: u16,
    preferred_effect: u32,
}

unsafe extern "system" fn data_object_query_interface(
    this: *mut c_void,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> i32 {
    if ppv.is_null() {
        return E_POINTER;
    }
    if guid_eq(riid, &IID_IUNKNOWN) || guid_eq(riid, &IID_IDATAOBJECT) {
        let obj = &*(this as *mut RovikDataObject);
        obj.ref_count.fetch_add(1, Ordering::SeqCst);
        *ppv = this;
        S_OK
    } else {
        *ppv = ptr::null_mut();
        E_NOINTERFACE
    }
}

unsafe extern "system" fn data_object_add_ref(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikDataObject);
    obj.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn data_object_release(this: *mut c_void) -> u32 {
    let obj = &*(this as *mut RovikDataObject);
    let old = obj.ref_count.fetch_sub(1, Ordering::SeqCst);
    if old == 1 {
        drop(Box::from_raw(this as *mut RovikDataObject));
        0
    } else {
        old - 1
    }
}

unsafe extern "system" fn data_object_get_data(
    this: *mut c_void,
    pformatetc: *const FORMATETC,
    pmedium: *mut STGMEDIUM,
) -> i32 {
    if pformatetc.is_null() || pmedium.is_null() {
        return E_INVALIDARG;
    }
    let fetc = &*pformatetc;
    if (fetc.tymed & TYMED_HGLOBAL) == 0 {
        return DV_E_FORMATETC;
    }

    let obj = &*(this as *mut RovikDataObject);

    if fetc.cfFormat == CF_HDROP {
        let h_mem = create_hdrop_global(&obj.paths);
        if h_mem.is_null() {
            return E_OUTOFMEMORY;
        }
        (*pmedium).tymed = TYMED_HGLOBAL;
        (*pmedium).hGlobal = h_mem;
        (*pmedium).pUnkForRelease = ptr::null_mut();
        return S_OK;
    }

    if obj.format_drop_effect != 0 && fetc.cfFormat == obj.format_drop_effect {
        let h_mem = create_drop_effect_global(obj.preferred_effect);
        if h_mem.is_null() {
            return E_OUTOFMEMORY;
        }
        (*pmedium).tymed = TYMED_HGLOBAL;
        (*pmedium).hGlobal = h_mem;
        (*pmedium).pUnkForRelease = ptr::null_mut();
        return S_OK;
    }

    DV_E_FORMATETC
}

unsafe extern "system" fn data_object_get_data_here(
    _this: *mut c_void,
    _pformatetc: *const FORMATETC,
    _pmedium: *mut STGMEDIUM,
) -> i32 {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_query_get_data(
    this: *mut c_void,
    pformatetc: *const FORMATETC,
) -> i32 {
    if pformatetc.is_null() {
        return E_INVALIDARG;
    }
    let fetc = &*pformatetc;
    if (fetc.tymed & TYMED_HGLOBAL) == 0 {
        return DV_E_FORMATETC;
    }
    let obj = &*(this as *mut RovikDataObject);
    if fetc.cfFormat == CF_HDROP
        || (obj.format_drop_effect != 0 && fetc.cfFormat == obj.format_drop_effect)
    {
        S_OK
    } else {
        DV_E_FORMATETC
    }
}

unsafe extern "system" fn data_object_get_canonical_format_etc(
    _this: *mut c_void,
    _in: *const FORMATETC,
    out: *mut FORMATETC,
) -> i32 {
    if out.is_null() {
        return E_POINTER;
    }
    (*out).ptd = ptr::null_mut();
    0x00040130 // DATA_S_SAMEFORMATETC
}

unsafe extern "system" fn data_object_set_data(
    _this: *mut c_void,
    _pformatetc: *const FORMATETC,
    _pmedium: *const STGMEDIUM,
    _release: i32,
) -> i32 {
    E_NOTIMPL
}

unsafe extern "system" fn data_object_enum_format_etc(
    this: *mut c_void,
    direction: u32,
    ppenum: *mut *mut c_void,
) -> i32 {
    if ppenum.is_null() {
        return E_POINTER;
    }
    if direction != DATADIR_GET {
        return E_NOTIMPL;
    }

    let obj = &*(this as *mut RovikDataObject);
    let mut formats = vec![FORMATETC {
        cfFormat: CF_HDROP,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL,
    }];

    if obj.format_drop_effect != 0 {
        formats.push(FORMATETC {
            cfFormat: obj.format_drop_effect,
            ptd: ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT,
            lindex: -1,
            tymed: TYMED_HGLOBAL,
        });
    }

    let enum_obj = Box::new(RovikEnumFormatEtc {
        lpVtbl: &ENUM_FORMATETC_VTBL,
        ref_count: AtomicU32::new(1),
        formats,
        index: 0,
    });
    *ppenum = Box::into_raw(enum_obj) as *mut c_void;
    S_OK
}

unsafe extern "system" fn data_object_d_advise(
    _this: *mut c_void,
    _pformatetc: *const FORMATETC,
    _advf: u32,
    _advsink: *mut c_void,
    _connection: *mut u32,
) -> i32 {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_object_d_unadvise(_this: *mut c_void, _connection: u32) -> i32 {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn data_object_enum_d_advise(
    _this: *mut c_void,
    _ppenum: *mut *mut c_void,
) -> i32 {
    OLE_E_ADVISENOTSUPPORTED
}

// ---------------------------------------------------------------------------
// Helpers for HGLOBAL creation
// ---------------------------------------------------------------------------

fn create_hdrop_global(paths: &[PathBuf]) -> *mut c_void {
    if paths.is_empty() {
        return ptr::null_mut();
    }

    let mut wide_chars: Vec<u16> = Vec::new();
    for p in paths {
        for c in p.as_os_str().encode_wide() {
            wide_chars.push(c);
        }
        wide_chars.push(0);
    }
    wide_chars.push(0); // Double null-terminator

    let dropfiles_size = std::mem::size_of::<DROPFILES>();
    let total_bytes = dropfiles_size + wide_chars.len() * 2;

    let h_mem = unsafe { GlobalAlloc(GMEM_MOVEABLE | 0x0040 /* GMEM_ZEROINIT */, total_bytes) };
    if h_mem.is_null() {
        return ptr::null_mut();
    }

    let p = unsafe { GlobalLock(h_mem) as *mut u8 };
    if p.is_null() {
        unsafe { GlobalFree(h_mem) };
        return ptr::null_mut();
    }

    unsafe {
        let df = p as *mut DROPFILES;
        (*df).pFiles = dropfiles_size as u32;
        (*df).pt = POINT { x: 0, y: 0 };
        (*df).fNC = 0;
        (*df).fWide = 1;

        let target_slice =
            std::slice::from_raw_parts_mut(p.add(dropfiles_size) as *mut u16, wide_chars.len());
        target_slice.copy_from_slice(&wide_chars);

        GlobalUnlock(h_mem);
    }

    h_mem as *mut c_void
}

fn create_drop_effect_global(effect: u32) -> *mut c_void {
    let h_mem = unsafe { GlobalAlloc(GMEM_MOVEABLE | 0x0040, 4) };
    if h_mem.is_null() {
        return ptr::null_mut();
    }
    let p = unsafe { GlobalLock(h_mem) as *mut u32 };
    if !p.is_null() {
        unsafe {
            *p = effect;
            GlobalUnlock(h_mem);
        }
    }
    h_mem as *mut c_void
}

// ---------------------------------------------------------------------------
// Entry Point: Start OLE Drag and Drop
// ---------------------------------------------------------------------------

/// Initiates an OLE Drag and Drop operation with the given filesystem paths.
/// Returns (hresult, final_effect, shift_pressed_at_drop).
pub fn start_drag_drop(_hwnd: HWND, paths: Vec<PathBuf>) -> (i32, u32, bool) {
    if paths.is_empty() {
        return (DRAGDROP_S_CANCEL, DROPEFFECT_NONE, false);
    }

    let format_drop_effect = unsafe {
        RegisterClipboardFormatW(crate::app::wide("Preferred DropEffect").as_ptr()) as u16
    };

    let shift_initially_down = unsafe { (GetKeyState(VK_SHIFT as i32) as u16 & 0x8000) != 0 };
    let preferred_effect = if shift_initially_down {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    };

    let data_obj = Box::new(RovikDataObject {
        lpVtbl: &DATA_OBJECT_VTBL,
        ref_count: AtomicU32::new(1),
        paths,
        format_drop_effect,
        preferred_effect,
    });

    let drop_src = Box::new(RovikDropSource {
        lpVtbl: &DROP_SOURCE_VTBL,
        ref_count: AtomicU32::new(1),
        shift_pressed_at_drop: AtomicBool::new(shift_initially_down),
    });

    let p_data = Box::into_raw(data_obj);
    let p_src = Box::into_raw(drop_src);

    let ok_effects = DROPEFFECT_COPY | DROPEFFECT_MOVE;
    let mut final_effect = DROPEFFECT_NONE;

    let hr = unsafe {
        DoDragDrop(
            p_data as *mut c_void,
            p_src as *mut c_void,
            ok_effects,
            &mut final_effect,
        )
    };

    let shift_at_drop = unsafe { (*p_src).shift_pressed_at_drop.load(Ordering::SeqCst) };

    // Release our root reference (if target held refs, it will free upon target release)
    unsafe {
        ((*(*p_data).lpVtbl).Release)(p_data as *mut c_void);
        ((*(*p_src).lpVtbl).Release)(p_src as *mut c_void);
    }

    (hr, final_effect, shift_at_drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guid_equality() {
        assert!(guid_eq(&IID_IUNKNOWN, &IID_IUNKNOWN));
        assert!(guid_eq(&IID_IDATAOBJECT, &IID_IDATAOBJECT));
        assert!(!guid_eq(&IID_IUNKNOWN, &IID_IDATAOBJECT));
    }

    #[test]
    fn test_data_object_creation_and_refcount() {
        let paths = vec![PathBuf::from("C:\\test\\file.txt")];
        let obj = Box::new(RovikDataObject {
            lpVtbl: &DATA_OBJECT_VTBL,
            ref_count: AtomicU32::new(1),
            paths,
            format_drop_effect: 0,
            preferred_effect: DROPEFFECT_COPY,
        });
        let raw = Box::into_raw(obj) as *mut c_void;

        unsafe {
            assert_eq!(data_object_add_ref(raw), 2);
            assert_eq!(data_object_release(raw), 1);
            assert_eq!(data_object_release(raw), 0);
        }
    }

    #[test]
    fn test_drop_source_query_continue() {
        let src = Box::new(RovikDropSource {
            lpVtbl: &DROP_SOURCE_VTBL,
            ref_count: AtomicU32::new(1),
            shift_pressed_at_drop: AtomicBool::new(false),
        });
        let raw = Box::into_raw(src) as *mut c_void;

        unsafe {
            // Escape pressed -> cancel
            assert_eq!(drop_source_query_continue_drag(raw, 1, MK_LBUTTON), DRAGDROP_S_CANCEL);

            // Left button released -> drop!
            assert_eq!(drop_source_query_continue_drag(raw, 0, 0), DRAGDROP_S_DROP);
            assert_eq!((* (raw as *mut RovikDropSource)).shift_pressed_at_drop.load(Ordering::SeqCst), false);

            // Left button released with Shift -> drop with Shift!
            assert_eq!(drop_source_query_continue_drag(raw, 0, MK_SHIFT), DRAGDROP_S_DROP);
            assert_eq!((* (raw as *mut RovikDropSource)).shift_pressed_at_drop.load(Ordering::SeqCst), true);

            // Left button still held down -> S_OK (continue drag)
            assert_eq!(drop_source_query_continue_drag(raw, 0, MK_LBUTTON), S_OK);

            drop_source_release(raw);
        }
    }

    #[test]
    fn test_data_object_get_data_and_enum() {
        let paths = vec![PathBuf::from("C:\\test\\demo.txt")];
        let obj = Box::new(RovikDataObject {
            lpVtbl: &DATA_OBJECT_VTBL,
            ref_count: AtomicU32::new(1),
            paths,
            format_drop_effect: 49152,
            preferred_effect: DROPEFFECT_COPY,
        });
        let raw = Box::into_raw(obj) as *mut c_void;

        unsafe {
            // QueryGetData valid CF_HDROP
            let fetc_valid = FORMATETC {
                cfFormat: CF_HDROP,
                ptd: ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT,
                lindex: -1,
                tymed: TYMED_HGLOBAL,
            };
            assert_eq!(data_object_query_get_data(raw, &fetc_valid), S_OK);

            // QueryGetData invalid format
            let fetc_invalid = FORMATETC {
                cfFormat: 9999,
                ptd: ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT,
                lindex: -1,
                tymed: TYMED_HGLOBAL,
            };
            assert_eq!(data_object_query_get_data(raw, &fetc_invalid), DV_E_FORMATETC);

            // GetData CF_HDROP
            let mut stg: STGMEDIUM = std::mem::zeroed();
            assert_eq!(data_object_get_data(raw, &fetc_valid, &mut stg), S_OK);
            assert_eq!(stg.tymed, TYMED_HGLOBAL);
            assert!(!stg.hGlobal.is_null());
            GlobalFree(stg.hGlobal);

            // EnumFormatEtc
            let mut p_enum: *mut c_void = ptr::null_mut();
            assert_eq!(data_object_enum_format_etc(raw, DATADIR_GET, &mut p_enum), S_OK);
            assert!(!p_enum.is_null());

            let mut items = [FORMATETC {
                cfFormat: 0,
                ptd: ptr::null_mut(),
                dwAspect: 0,
                lindex: 0,
                tymed: 0,
            }; 3];
            let mut fetched = 0u32;
            let hr = enum_formatetc_next(p_enum, 3, items.as_mut_ptr(), &mut fetched);
            assert_eq!(hr, S_FALSE); // Less than requested 3
            assert_eq!(fetched, 2);  // CF_HDROP + Preferred DropEffect
            assert_eq!(items[0].cfFormat, CF_HDROP);
            assert_eq!(items[1].cfFormat, 49152);

            enum_formatetc_release(p_enum);
            data_object_release(raw);
        }
    }
}
