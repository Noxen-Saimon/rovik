#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("Rovik requires Windows.");
}

#[cfg(windows)]
mod app;
#[cfg(windows)]
mod archive;
#[cfg(windows)]
mod fs;
#[cfg(windows)]
mod preview;
#[cfg(windows)]
mod settings;

#[cfg(windows)]
fn main() {
    app::run();
}
