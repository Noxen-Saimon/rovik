# TASK: ROVIK FILE EXPLORER - CORE ENGINE FOUNDATION

## Objective
Build the core backend engine for "Rovik", a zero-bloat, hyper-lightweight File Explorer alternative on Windows 11 using Rust and raw Windows API (`windows-sys`). Target: < 10 MB RAM, near-zero idle CPU usage.

## Technical Constraints
1. Language & Crates: Rust 2021 edition. Strictly use `windows-sys` (v0.59+) only. No heavy frameworks, no external shell libraries, no COM overhead.
2. Cargo Profile Optimizations:
   - `opt-level = "z"`, `lto = true`, `panic = "abort"`, `codegen-units = 1`, `strip = true`.
3. Memory & Performance Rules:
   - Zero unnecessary allocations: Avoid dynamic string cloning inside directory traversal loops. Reuse buffers.
   - Native Wide Char: Use Windows UTF-16 APIs directly (`FindFirstFileW`, `FindNextFileW`, `FindClose`). Never convert back and forth between ANSI and UTF-16 unless passing to UI.
   - Bypass all Windows Search indexers and OneDrive integration.

## Modules to Implement
1. `src/fs/engine.rs`:
   - Fast direct directory enumerator returning a compact struct (`name`, `size`, `modified_time`, `is_dir`, `is_hidden`).
   - Filter handling: toggle to show/hide hidden items.
2. `src/fs/search.rs`:
   - Recursive asynchronous or worker-threaded instant search by filename/extension.
   - Non-blocking search traversal that can stream results directly to a buffer.
3. `src/fs/nav.rs`:
   - Path parsing, sanitization, history stack (Back/Forward), and parent directory resolution (`..`).

## Output Requirements
Generate clean, idiomatic, and minimal-unsafe Rust code for the foundation modules. Code only, no verbose explanations.