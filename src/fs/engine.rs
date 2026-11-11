use std::path::Path;
use windows_sys::Win32::{
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{
        FindClose, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_HIDDEN,
        WIN32_FIND_DATAW,
    },
};

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: Vec<u16>,
    pub size: u64,
    pub modified_time: u64,
    pub is_dir: bool,
    pub is_hidden: bool,
}

impl Entry {
    #[inline]
    pub fn name_lossy(&self) -> String {
        String::from_utf16_lossy(&self.name)
    }

    #[inline]
    pub fn extension_u16(&self) -> Option<&[u16]> {
        self.name
            .iter()
            .rposition(|&c| c == b'.' as u16)
            .map(|idx| &self.name[idx + 1..])
    }
}

pub fn enumerate(path: &Path, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    use std::os::windows::ffi::OsStrExt;
    let mut search_path: Vec<u16> = path.as_os_str().encode_wide().collect();
    enumerate_wide(&mut search_path, show_hidden)
}

pub fn enumerate_wide(search_path: &mut Vec<u16>, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    let original_len = search_path.len();
    if search_path.is_empty() {
        return Ok(Vec::new());
    }

    let last = search_path[original_len - 1];
    if last != b'\\' as u16 && last != b'/' as u16 {
        search_path.push(b'\\' as u16);
    }
    search_path.push(b'*' as u16);
    search_path.push(0);

    let mut data = unsafe { std::mem::zeroed::<WIN32_FIND_DATAW>() };
    let handle = unsafe { FindFirstFileW(search_path.as_ptr(), &mut data) };

    search_path.truncate(original_len);

    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }

    let mut entries = Vec::with_capacity(64);

    loop {
        let c0 = data.cFileName[0];
        let c1 = data.cFileName[1];
        let c2 = data.cFileName[2];

        let is_dot = c0 == b'.' as u16 && c1 == 0;
        let is_dotdot = c0 == b'.' as u16 && c1 == b'.' as u16 && c2 == 0;

        if !is_dot && !is_dotdot {
            let is_hidden = (data.dwFileAttributes & FILE_ATTRIBUTE_HIDDEN) != 0;
            if show_hidden || !is_hidden {
                let is_dir = (data.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
                let size = ((data.nFileSizeHigh as u64) << 32) | (data.nFileSizeLow as u64);
                let modified_time = ((data.ftLastWriteTime.dwHighDateTime as u64) << 32)
                    | (data.ftLastWriteTime.dwLowDateTime as u64);

                let len = data
                    .cFileName
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(data.cFileName.len());

                let mut name = Vec::with_capacity(len);
                name.extend_from_slice(&data.cFileName[..len]);

                entries.push(Entry {
                    name,
                    size,
                    modified_time,
                    is_dir,
                    is_hidden,
                });
            }
        }

        if unsafe { FindNextFileW(handle, &mut data) } == 0 {
            break;
        }
    }

    unsafe { FindClose(handle) };

    entries.sort_unstable_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| cmp_u16_ci(&a.name, &b.name))
    });

    Ok(entries)
}

#[inline]
pub fn cmp_u16_ci(a: &[u16], b: &[u16]) -> std::cmp::Ordering {
    for (&ca, &cb) in a.iter().zip(b.iter()) {
        let la = to_lower_u16(ca);
        let lb = to_lower_u16(cb);
        if la != lb {
            return la.cmp(&lb);
        }
    }
    a.len().cmp(&b.len())
}

#[inline]
pub fn to_lower_u16(c: u16) -> u16 {
    if (b'A' as u16..=b'Z' as u16).contains(&c) {
        c + 32
    } else {
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_system_directory() {
        let win = Path::new(r#"C:\Windows"#);
        if win.is_dir() {
            let entries = enumerate(win, false).expect("enumerate C:\\Windows");
            assert!(!entries.is_empty());
            // Verify sorted directories first
            let mut seen_file = false;
            for entry in entries {
                if !entry.is_dir {
                    seen_file = true;
                } else if seen_file {
                    panic!("Found folder after file in sorted entries: {}", entry.name_lossy());
                }
            }
        }
    }

    #[test]
    fn test_cmp_u16_ci() {
        let a: Vec<u16> = "AbCdE".encode_utf16().collect();
        let b: Vec<u16> = "abcde".encode_utf16().collect();
        assert_eq!(cmp_u16_ci(&a, &b), std::cmp::Ordering::Equal);

        let c: Vec<u16> = "abcdF".encode_utf16().collect();
        assert_eq!(cmp_u16_ci(&a, &c), std::cmp::Ordering::Less);
    }
}
