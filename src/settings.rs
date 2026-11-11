//! Persistent user settings for Rovik.
//!
//! Stored as a tiny human-readable `key=value` file at
//! `%APPDATA%\Rovik\settings.ini`. Unknown keys are ignored and missing keys
//! fall back to defaults, so the format stays forward/backward compatible.

use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartupMode {
    Home,
    LastSession,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Editor {
    Zed,
    VSCodium,
    Notepad,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terminal {
    PowerShell,
    WindowsTerminal,
    Cmd,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewMode {
    Details,
    Compact,
    Grid,
}

impl StartupMode {
    pub const ALL: [StartupMode; 3] = [StartupMode::Home, StartupMode::LastSession, StartupMode::Custom];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            StartupMode::Home => "Home folder",
            StartupMode::LastSession => "Restore last session",
            StartupMode::Custom => "Custom folder",
        }
    }

    fn key(self) -> &'static str {
        match self {
            StartupMode::Home => "home",
            StartupMode::LastSession => "last_session",
            StartupMode::Custom => "custom",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }
}

impl Editor {
    pub const ALL: [Editor; 4] = [Editor::Zed, Editor::VSCodium, Editor::Notepad, Editor::Custom];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Editor::Zed => "Zed",
            Editor::VSCodium => "VSCodium",
            Editor::Notepad => "Notepad",
            Editor::Custom => "Custom editor",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Editor::Zed => "zed",
            Editor::VSCodium => "vscodium",
            Editor::Notepad => "notepad",
            Editor::Custom => "custom",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }
}

impl Terminal {
    pub const ALL: [Terminal; 3] = [Terminal::PowerShell, Terminal::WindowsTerminal, Terminal::Cmd];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Terminal::PowerShell => "PowerShell",
            Terminal::WindowsTerminal => "Windows Terminal",
            Terminal::Cmd => "Command Prompt",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Terminal::PowerShell => "powershell",
            Terminal::WindowsTerminal => "wt",
            Terminal::Cmd => "cmd",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }
}

impl ViewMode {
    pub const ALL: [ViewMode; 3] = [ViewMode::Details, ViewMode::Compact, ViewMode::Grid];

    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Details => "Detailed List",
            ViewMode::Compact => "Compact List",
            ViewMode::Grid => "Grid / Icons",
        }
    }

    fn key(self) -> &'static str {
        match self {
            ViewMode::Details => "details",
            ViewMode::Compact => "compact",
            ViewMode::Grid => "grid",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == s)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Settings {
    // General
    pub startup_mode: StartupMode,
    pub startup_folder: String,

    // View
    pub view_mode: ViewMode,
    pub show_hidden: bool,
    pub show_extensions: bool,
    pub folders_first: bool,

    // File operations
    pub recycle_bin: bool,
    pub confirm_delete: bool,
    pub warn_ext_change: bool,
    pub rename_select_stem: bool,

    // Apps
    pub editor: Editor,
    pub editor_custom_path: String,
    pub open_text_in_editor: bool,
    pub terminal: Terminal,

    // Pinned Quick Access folders
    pub pinned_folders: Vec<String>,

    // Session (not shown in the UI; written automatically on exit)
    pub last_session: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            startup_mode: StartupMode::Home,
            startup_folder: String::new(),
            view_mode: ViewMode::Details,
            show_hidden: false,
            show_extensions: true,
            folders_first: true,
            recycle_bin: true,
            confirm_delete: true,
            warn_ext_change: true,
            rename_select_stem: true,
            editor: Editor::Zed,
            editor_custom_path: String::new(),
            open_text_in_editor: false,
            terminal: Terminal::PowerShell,
            pinned_folders: Vec::new(),
            last_session: Vec::new(),
        }
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// Parses settings text. Never fails: invalid or unknown lines are skipped.
    pub fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') || line.starts_with('[') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim();
            let value = value.trim();

            match key {
                "startup_mode" => {
                    if let Some(m) = StartupMode::from_key(value) {
                        s.startup_mode = m;
                    }
                }
                "startup_folder" => s.startup_folder = value.to_string(),
                "view_mode" => {
                    if let Some(vm) = ViewMode::from_key(value) {
                        s.view_mode = vm;
                    }
                }
                "editor" => {
                    if let Some(e) = Editor::from_key(value) {
                        s.editor = e;
                    }
                }
                "editor_custom_path" => s.editor_custom_path = value.to_string(),
                "terminal" => {
                    if let Some(t) = Terminal::from_key(value) {
                        s.terminal = t;
                    }
                }
                // '|' can never appear in a Windows path, so it is a safe separator.
                "pinned_folders" => {
                    s.pinned_folders = value
                        .split('|')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(String::from)
                        .collect();
                }
                "last_session" => {
                    s.last_session = value
                        .split('|')
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                        .map(String::from)
                        .collect();
                }
                _ => {
                    let Some(b) = parse_bool(value) else { continue };
                    match key {
                        "show_hidden" => s.show_hidden = b,
                        "show_extensions" => s.show_extensions = b,
                        "folders_first" => s.folders_first = b,
                        "recycle_bin" => s.recycle_bin = b,
                        "confirm_delete" => s.confirm_delete = b,
                        "warn_ext_change" => s.warn_ext_change = b,
                        "rename_select_stem" => s.rename_select_stem = b,
                        "open_text_in_editor" => s.open_text_in_editor = b,
                        _ => {}
                    }
                }
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let b = |v: bool| if v { "true" } else { "false" };
        let mut out = String::new();
        out.push_str("# Rovik settings. Edit from the app: Settings (Ctrl+,)\r\n");
        out.push_str("[general]\r\n");
        out.push_str(&format!("startup_mode={}\r\n", self.startup_mode.key()));
        out.push_str(&format!("startup_folder={}\r\n", self.startup_folder));
        out.push_str("[view]\r\n");
        out.push_str(&format!("view_mode={}\r\n", self.view_mode.key()));
        out.push_str(&format!("show_hidden={}\r\n", b(self.show_hidden)));
        out.push_str(&format!("show_extensions={}\r\n", b(self.show_extensions)));
        out.push_str(&format!("folders_first={}\r\n", b(self.folders_first)));
        out.push_str("[file_operations]\r\n");
        out.push_str(&format!("recycle_bin={}\r\n", b(self.recycle_bin)));
        out.push_str(&format!("confirm_delete={}\r\n", b(self.confirm_delete)));
        out.push_str(&format!("warn_ext_change={}\r\n", b(self.warn_ext_change)));
        out.push_str(&format!("rename_select_stem={}\r\n", b(self.rename_select_stem)));
        out.push_str("[apps]\r\n");
        out.push_str(&format!("editor={}\r\n", self.editor.key()));
        out.push_str(&format!("editor_custom_path={}\r\n", self.editor_custom_path));
        out.push_str(&format!("open_text_in_editor={}\r\n", b(self.open_text_in_editor)));
        out.push_str(&format!("terminal={}\r\n", self.terminal.key()));
        out.push_str("[quick_access]\r\n");
        out.push_str(&format!("pinned_folders={}\r\n", self.pinned_folders.join("|")));
        out.push_str("[session]\r\n");
        out.push_str(&format!("last_session={}\r\n", self.last_session.join("|")));
        out
    }

    pub fn file_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("Rovik").join("settings.ini"))
    }

    pub fn load() -> Self {
        Self::file_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|t| Self::parse(&t))
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::file_path()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "APPDATA is not set"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ini.tmp");
        std::fs::write(&tmp, self.serialize())?;
        std::fs::rename(&tmp, &path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_all_fields() {
        let s = Settings {
            startup_mode: StartupMode::Custom,
            startup_folder: r"D:\Projects".into(),
            view_mode: ViewMode::Grid,
            show_hidden: true,
            show_extensions: false,
            folders_first: false,
            recycle_bin: false,
            confirm_delete: false,
            warn_ext_change: false,
            rename_select_stem: false,
            editor: Editor::Custom,
            editor_custom_path: r"C:\Tools\Editor\edit.exe".into(),
            open_text_in_editor: true,
            terminal: Terminal::WindowsTerminal,
            pinned_folders: vec![r"D:\Projects".into(), r"C:\Work".into()],
            last_session: vec![r"C:\Users".into(), r"E:\".into()],
        };
        assert_eq!(Settings::parse(&s.serialize()), s);
    }

    #[test]
    fn defaults_for_missing_or_invalid_values() {
        let s = Settings::parse("editor=unknown\nshow_hidden=maybe\ngarbage line\n");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn empty_session_parses_to_empty_list() {
        let s = Settings::parse("last_session=\n");
        assert!(s.last_session.is_empty());
    }
}
