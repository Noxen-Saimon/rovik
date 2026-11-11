use crate::fs::engine::{to_lower_u16, Entry};
use std::{
    ffi::OsString,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
};
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{
        FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
        FILE_ATTRIBUTE_REPARSE_POINT, WIN32_FIND_DATAW,
    },
};

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub full_path: PathBuf,
    pub entry: Entry,
}

pub struct SearchHandle {
    pub receiver: Receiver<SearchResult>,
    pub cancel: Arc<AtomicBool>,
}

impl SearchHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn search(root: PathBuf, query_str: &str, show_hidden: bool) -> SearchHandle {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_clone = Arc::clone(&cancel);

    let query_u16: Vec<u16> = query_str.encode_utf16().collect();
    let is_ext_query = query_str.starts_with('.');
    let query_lower: Vec<u16> = query_u16.iter().map(|&c| to_lower_u16(c)).collect();

    thread::spawn(move || {
        let mut stack: Vec<PathBuf> = vec![root];
        let mut path_buf: Vec<u16> = Vec::with_capacity(512);

        while let Some(current_dir) = stack.pop() {
            if cancel_clone.load(Ordering::Relaxed) {
                return;
            }

            path_buf.clear();
            path_buf.extend(current_dir.as_os_str().encode_wide());
            let dir_len = path_buf.len();

            if dir_len == 0 {
                continue;
            }

            if path_buf[dir_len - 1] != b'\\' as u16 && path_buf[dir_len - 1] != b'/' as u16 {
                path_buf.push(b'\\' as u16);
            }
            path_buf.push(b'*' as u16);
            path_buf.push(0);

            let mut data = unsafe { std::mem::zeroed::<WIN32_FIND_DATAW>() };
            let handle = unsafe { FindFirstFileW(path_buf.as_ptr(), &mut data) };

            if handle == INVALID_HANDLE_VALUE {
                continue;
            }

            loop {
                if cancel_clone.load(Ordering::Relaxed) {
                    unsafe { FindClose(handle) };
                    return;
                }

                let c0 = data.cFileName[0];
                let c1 = data.cFileName[1];
                let c2 = data.cFileName[2];

                let is_dot = c0 == b'.' as u16 && c1 == 0;
                let is_dotdot = c0 == b'.' as u16 && c1 == b'.' as u16 && c2 == 0;

                if !is_dot && !is_dotdot {
                    let is_hidden = (data.dwFileAttributes & FILE_ATTRIBUTE_HIDDEN) != 0;
                    let is_dir = (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                    let is_reparse = (data.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0;

                    if show_hidden || !is_hidden {
                        let len = data
                            .cFileName
                            .iter()
                            .position(|&c| c == 0)
                            .unwrap_or(data.cFileName.len());
                        let name_slice = &data.cFileName[..len];

                        let matches = if is_ext_query {
                            match name_slice.iter().rposition(|&c| c == b'.' as u16) {
                                Some(dot_idx) => {
                                    let ext = &name_slice[dot_idx..];
                                    match_slice_ci(ext, &query_lower)
                                }
                                None => false,
                            }
                        } else {
                            contains_slice_ci(name_slice, &query_lower)
                        };

                        let child_path = current_dir.join(OsString::from_wide(name_slice));

                        if matches {
                            let size =
                                ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64);
                            let modified_time =
                                ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                                    | (data.ftLastWriteTime.dwLowDateTime as u64);

                            let item = SearchResult {
                                full_path: child_path.clone(),
                                entry: Entry {
                                    name: name_slice.to_vec(),
                                    size,
                                    modified_time,
                                    is_dir,
                                    is_hidden,
                                },
                            };

                            if tx.send(item).is_err() {
                                unsafe { FindClose(handle) };
                                return;
                            }
                        }

                        if is_dir && !is_reparse {
                            stack.push(child_path);
                        }
                    }
                }

                if unsafe { FindNextFileW(handle, &mut data) } == 0 {
                    break;
                }
            }

            unsafe { FindClose(handle) };
        }
    });

    SearchHandle {
        receiver: rx,
        cancel,
    }
}

#[inline]
fn match_slice_ci(a: &[u16], b_lower: &[u16]) -> bool {
    if a.len() != b_lower.len() {
        return false;
    }
    a.iter()
        .zip(b_lower.iter())
        .all(|(&ca, &cb)| to_lower_u16(ca) == cb)
}

#[inline]
fn contains_slice_ci(haystack: &[u16], needle_lower: &[u16]) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.len() < needle_lower.len() {
        return false;
    }
    haystack.windows(needle_lower.len()).any(|w| {
        w.iter()
            .zip(needle_lower.iter())
            .all(|(&h, &n)| to_lower_u16(h) == n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_streaming() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let handle = search(root, "Cargo.toml", true);
        let mut found = false;
        // Search should find Cargo.toml
        while let Ok(res) = handle.receiver.recv_timeout(std::time::Duration::from_millis(500)) {
            if res.entry.name_lossy().eq_ignore_ascii_case("Cargo.toml") {
                found = true;
                break;
            }
        }
        handle.cancel();
        assert!(found, "Should stream Cargo.toml from search");
    }
}
