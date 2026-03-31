//! GridSnap ライブラリクレート。
//! main.rs からは `use gridsnap::*` でアクセスし、
//! tests/ ディレクトリの統合テストからも `use gridsnap::*` でアクセス可能。

pub mod config;
pub mod grid;

// ──── Windows modules ────
#[cfg(target_os = "windows")]
pub mod monitor;
#[cfg(target_os = "windows")]
pub mod snap;
#[cfg(target_os = "windows")]
pub mod event_hook;
#[cfg(target_os = "windows")]
pub mod overlay;
#[cfg(target_os = "windows")]
pub mod auto_place;
#[cfg(target_os = "windows")]
pub mod titlebar;
#[cfg(target_os = "windows")]
pub mod tray;
#[cfg(target_os = "windows")]
pub mod startup;

// ──── macOS modules ────
#[cfg(target_os = "macos")]
pub mod mac_ffi;
#[cfg(target_os = "macos")]
pub mod mac_monitor;
#[cfg(target_os = "macos")]
pub mod mac_snap;
#[cfg(target_os = "macos")]
pub mod mac_overlay;
#[cfg(target_os = "macos")]
pub mod mac_event_hook;
#[cfg(target_os = "macos")]
pub mod mac_tray;
#[cfg(target_os = "macos")]
pub mod mac_startup;