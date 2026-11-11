# Rovik 📁⚡

> Ultra-lightweight, zero-bloat Windows File Explorer alternative built with Rust and native Win32 API (`windows-sys`).

Rovik is designed for speed, minimal resource consumption, and a distraction-free file browsing experience on Windows 10 and 11. Built entirely without heavy web wrappers (Electron/Tauri) or high-overhead shell abstractions, Rovik runs natively on the Windows GDI subsystem with sub-10 MB memory usage and near-zero idle CPU.

---

## 🚀 Key Features

* **Instant Native Performance:** Direct UTF-16 Windows API traversal (`FindFirstFileW`, `FindNextFileW`) bypassing Windows Search indexing overhead and background telemetry.
* **Flicker-Free Custom UI:** Handcrafted double-buffered GDI rendering with Per-Monitor DPI V2 awareness and smooth scrolling.
* **Muted Slate & Warm Sand Palette:** Calm, high-contrast, distraction-free visual design tuned for readability without harsh neon themes.
* **Instant Path Navigation & Search:** Direct editable address bar (`Ctrl + L`) and asynchronous, non-blocking file search.
* **File Operations & Batch Rename:** Built-in multi-file batch rename tool with find/replace, prefix/suffix, and number formatting.
* **Integrated File Preview & Archive Viewer:** Quick text and media preview panel, plus native `.zip` inspection without external utilities.
* **Drag-and-Drop & Clipboard Integration:** Native OLE drag-and-drop and clipboard cut/copy/paste.
* **Default Explorer Integration:** Optional registry scripts to handle directory opening directly in Rovik.

---

## 📊 Technical Highlights & Constraints

| Metric / Aspect | Rovik | Standard Windows Explorer |
| :--- | :--- | :--- |
| **Idle Memory (RAM)** | **~6 – 10 MB** | 80 – 250+ MB |
| **Idle CPU** | **0.0%** | Periodic background activity |
| **GUI Subsystem** | Native Win32 GDI (Double-buffered) | XAML / WinUI 3 / AppX Islands |
| **Search Engine** | Direct directory stream worker | Windows Search Indexer service |
| **Binary Footprint** | Small single executable (LTO + Strip) | Multi-DLL OS package |

---

## ⌨️ Shortcuts & Keybindings

| Shortcut | Action |
| :--- | :--- |
| `Ctrl + L` / `Alt + D` | Focus address / path bar |
| `Backspace` / `Alt + Up` | Go to parent directory |
| `Alt + Left` / `Alt + Right` | Navigate Back / Forward in history |
| `F2` | Rename selected item |
| `F5` | Refresh current directory |
| `Ctrl + F` | Focus search bar |
| `Ctrl + H` | Toggle hidden files visibility |
| `Ctrl + M` | Open Batch Rename utility |
| `Enter` | Open file or enter directory |
| `Alt + Enter` | View item properties |

---

## 🛠️ Build & Installation

### Prerequisites
* **Windows 10 / 11 (64-bit)**
* **Rust toolchain** (stable 2021 edition):
  ```cmd
  rustup update stable
  ```

### Build from Source

```cmd
git clone https://github.com/Noxen-Saimon/rovik.git
cd rovik
cargo build --release
```

The optimized binary will be created at:
```text
target/release/rovik.exe
```

*Release profile uses `opt-level = "z"`, `lto = true`, `codegen-units = 1`, and `strip = true` for maximum binary size reduction.*

---

## ⚙️ Optional: Set as Default Folder Handler

Rovik includes registry utility scripts to associate folder opening:

* **Register as folder handler:** Run `register-default-explorer.reg` (Admin privileges required).
* **Restore default Windows Explorer:** Run `restore-default-explorer.reg`.

---

## 📜 License

MIT License. Feel free to use, modify, and contribute.
