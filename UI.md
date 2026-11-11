# TASK: ROVIK FILE EXPLORER - CUSTOM WIN32 GUI & DESIGN SYSTEM

## Objective
Implement a lightweight, flicker-free native Win32 GUI for "Rovik" using pure Win32 GDI (`windows-sys`). The interface must look modern, calm, and unique, strictly avoiding generic AI/cyberpunk dark-mode clichés.

## Visual Design Specification
1. Color Palette (Muted Slate & Warm Sand):
   - Main Window Background: RGB(30, 32, 36) [#1E2024]
   - Header / Top Navigation Bar: RGB(24, 26, 29) [#181A1D]
   - Row Selection / Hover: RGB(45, 50, 57) [#2D3239]
   - Primary Text (File & Folder Names): RGB(224, 220, 211) [#E0DCD3]
   - Secondary Text (Size, Date, Ext): RGB(140, 146, 156) [#8C929C]
   - Accent / Focused Path: RGB(126, 159, 142) [#7E9F8E]
2. Typography (Non-Terminal, Humanist Sans):
   - Primary: "Plus Jakarta Sans" (Fallback: "Satoshi" or native Windows "Candara").
   - Quality: Strict `CLEARTYPE_QUALITY` via `CreateFontW`, size 10-11pt (Normal 400 for rows, Semi-Bold 600 for headers).

## UI Layout & Rendering Requirements
1. Structure:
   - Top Bar: Instant editable text Path Bar (`Ctrl + L` focus), search input, and quick-toggle button for hidden files.
   - Main Area: Virtual/Owner-drawn file list view (Name, Type/Ext, Size, Date Modified).
2. Rendering Engine:
   - Handle `WM_PAINT` using GDI Double-Buffering (Memory DC + `CreateCompatibleBitmap` + `BitBlt`) to completely eliminate flickering during fast scrolling.
   - Handle `WM_ERASEBKGND` (return 1) to avoid OS default white/gray flashing.
   - Owner-draw / Custom-draw rows using `NM_CUSTOMDRAW` with the exact color palette specified.

## Output Requirements
Provide complete Rust code integrating the Win32 window message loop (`WndProc`), GDI double-buffering, and custom-drawn controls matching the style rules above.