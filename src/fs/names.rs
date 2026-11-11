//! File-name validation rules matching what Windows (NTFS / FAT / exFAT) accepts.

use std::path::Path;

pub const INVALID_CHARS: [char; 9] = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

#[derive(Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    InvalidChars,
    TrailingDotOrSpace,
    DotsOnly,
    Reserved(String),
    TooLong,
}

impl NameError {
    pub fn message(&self) -> String {
        match self {
            NameError::Empty => "The name cannot be empty.".into(),
            NameError::InvalidChars => "A name can't contain any of these characters:  \\ / : * ? \" < > |".into(),
            NameError::TrailingDotOrSpace => "A name can't end with a space or a period.".into(),
            NameError::DotsOnly => "A name can't consist only of periods.".into(),
            NameError::Reserved(n) => format!("\"{n}\" is a reserved Windows name. Please choose another name."),
            NameError::TooLong => "The name is too long (maximum 255 characters).".into(),
        }
    }
}

/// Validates a single path component (not a full path). The caller is expected
/// to have trimmed surrounding whitespace already.
pub fn validate_file_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.chars().any(|c| INVALID_CHARS.contains(&c) || (c as u32) < 0x20) {
        return Err(NameError::InvalidChars);
    }
    if name.chars().all(|c| c == '.') {
        return Err(NameError::DotsOnly);
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err(NameError::TrailingDotOrSpace);
    }
    if name.encode_utf16().count() > 255 {
        return Err(NameError::TooLong);
    }
    // "CON", "con.txt", "Nul .log" ... are all reserved device names.
    let base = name.split('.').next().unwrap_or("").trim_end();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(base)) {
        return Err(NameError::Reserved(base.to_string()));
    }
    Ok(())
}

/// Lower-cased extension (without the dot), `None` for names like ".gitignore" or "README".
pub fn extension_lower(name: &str) -> Option<String> {
    Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// True when renaming `old` to `new` would change (add, remove or alter) the file extension.
pub fn extension_changed(old: &str, new: &str) -> bool {
    extension_lower(old) != extension_lower(new)
}

/// Windows file names are case-insensitive.
pub fn same_name_ci(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Number of UTF-16 units of the part of `name` before its extension (used to
/// pre-select only the stem in the rename box, like Windows Explorer).
pub fn stem_len_utf16(name: &str) -> usize {
    match Path::new(name).file_stem() {
        Some(stem) if Path::new(name).extension().is_some() => stem.to_string_lossy().encode_utf16().count(),
        _ => name.encode_utf16().count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_names() {
        assert!(validate_file_name("report.txt").is_ok());
        assert!(validate_file_name(".gitignore").is_ok());
        assert!(validate_file_name("My Folder (2)").is_ok());
        assert!(validate_file_name("tài liệu.docx").is_ok());
        assert!(validate_file_name("console.log").is_ok());
    }

    #[test]
    fn rejects_invalid_names() {
        assert_eq!(validate_file_name(""), Err(NameError::Empty));
        assert_eq!(validate_file_name("a/b"), Err(NameError::InvalidChars));
        assert_eq!(validate_file_name("what?"), Err(NameError::InvalidChars));
        assert_eq!(validate_file_name("tab\tname"), Err(NameError::InvalidChars));
        assert_eq!(validate_file_name("name."), Err(NameError::TrailingDotOrSpace));
        assert_eq!(validate_file_name(".."), Err(NameError::DotsOnly));
        assert_eq!(validate_file_name("CON"), Err(NameError::Reserved("CON".into())));
        assert_eq!(validate_file_name("nul.txt"), Err(NameError::Reserved("nul".into())));
        assert_eq!(validate_file_name(&"a".repeat(256)), Err(NameError::TooLong));
    }

    #[test]
    fn extension_helpers() {
        assert!(!extension_changed("a.TXT", "b.txt"));
        assert!(extension_changed("a.txt", "a.md"));
        assert!(extension_changed("a.txt", "a"));
        assert!(!extension_changed(".gitignore", ".gitignore2"));
        assert_eq!(stem_len_utf16("photo.jpeg"), 5);
        assert_eq!(stem_len_utf16("archive.tar.gz"), 11);
        assert_eq!(stem_len_utf16(".env"), 4);
        assert_eq!(stem_len_utf16("README"), 6);
        assert!(same_name_ci("Readme.MD", "readme.md"));
    }
}
