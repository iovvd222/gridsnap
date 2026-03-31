//! macOS: スタートアップ自動登録（LaunchAgent plist）
//!
//! ~/Library/LaunchAgents/com.gridsnap.app.plist を生成し、
//! ログイン時に自動起動させる。既に同一パスで登録済みならスキップ。

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

const LABEL: &str = "com.gridsnap.app";

/// LaunchAgent plist を配置してスタートアップ登録する。
/// plist が既に存在すれば（パスが異なっていても）スキップする。
/// 初回起動時のみ登録し、ユーザーが手動で削除した意図を尊重する。
pub fn ensure_registered() -> Result<()> {
    let exe = std::env::current_exe().context("Failed to get current exe path")?;
    let exe_str = exe.to_string_lossy().to_string();
    let plist_path = plist_path()?;

    // plist が存在すれば初回登録済みとみなす（パス不一致でも上書きしない）
    if plist_path.exists() {
        log::info!("LaunchAgent already exists at {:?}, skipping", plist_path);
        return Ok(());
    }

    // ~/Library/LaunchAgents が無ければ作成
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {:?}", parent))?;
    }

    let plist_content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>
"#,
        label = LABEL,
        exe = exe_str,
    );

    fs::write(&plist_path, &plist_content)
        .with_context(|| format!("Failed to write plist: {:?}", plist_path))?;

    log::info!("LaunchAgent registered: {:?}", plist_path);
    Ok(())
}

/// スタートアップ登録を解除する（plist を削除）。
pub fn unregister() -> Result<()> {
    let plist_path = plist_path()?;
    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("Failed to remove plist: {:?}", plist_path))?;
        log::info!("LaunchAgent removed: {:?}", plist_path);
    } else {
        log::info!("LaunchAgent not found, nothing to remove.");
    }
    Ok(())
}

// ── internal ──

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", LABEL)))
}