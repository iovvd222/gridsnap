//! Windows: スタートアップ自動登録（HKCU\Software\Microsoft\Windows\CurrentVersion\Run）
//!
//! 起動時に自分自身の exe パスをレジストリに書き込む。
//! 既に同じパスで登録済みなら何もしない。

use anyhow::{Context, Result};
use std::path::PathBuf;
use windows::core::HSTRING;
use windows::Win32::System::Registry::{
    RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, RegCloseKey,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
    REG_OPTION_NON_VOLATILE,
};
use windows::Win32::Foundation::ERROR_SUCCESS;

const SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE_NAME: &str = "GridSnap";

/// 現在の exe パスをスタートアップに登録する。
/// 既に同一パスで登録済みならスキップする。
pub fn ensure_registered() -> Result<()> {
    let exe = std::env::current_exe().context("Failed to get current exe path")?;
    let exe_str = exe.to_string_lossy();

    // 既存値を確認
    if let Ok(existing) = read_current_value() {
        if existing == exe_str {
            log::info!("Startup already registered: {}", exe_str);
            return Ok(());
        }
    }

    write_value(&exe_str)?;
    log::info!("Registered startup entry: {}", exe_str);
    Ok(())
}

/// スタートアップ登録を解除する。
pub fn unregister() -> Result<()> {
    use windows::Win32::System::Registry::RegDeleteValueW;

    unsafe {
        let subkey = HSTRING::from(SUBKEY);
        let mut hkey = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegCreateKeyExW failed: {:?}", status);
        }

        let name = HSTRING::from(VALUE_NAME);
        let _ = RegDeleteValueW(hkey, &name);
        let _ = RegCloseKey(hkey);
    }
    log::info!("Startup entry removed.");
    Ok(())
}

// ── internal ──

fn read_current_value() -> Result<String> {
    unsafe {
        let subkey = HSTRING::from(SUBKEY);
        let mut hkey = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ,
            None,
            &mut hkey,
            None,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegCreateKeyExW (read) failed: {:?}", status);
        }

        let name = HSTRING::from(VALUE_NAME);
        let mut buf_size: u32 = 0;
        // 最初の呼び出しでサイズを取得
        let _ = RegQueryValueExW(
            hkey,
            &name,
            None,
            None,
            None,
            Some(&mut buf_size),
        );
        if buf_size == 0 {
            let _ = RegCloseKey(hkey);
            anyhow::bail!("Value not found or empty");
        }

        let mut buf = vec![0u8; buf_size as usize];
        let status = RegQueryValueExW(
            hkey,
            &name,
            None,
            None,
            Some(buf.as_mut_ptr()),
            Some(&mut buf_size),
        );
        let _ = RegCloseKey(hkey);

        if status != ERROR_SUCCESS {
            anyhow::bail!("RegQueryValueExW failed: {:?}", status);
        }

        // REG_SZ は UTF-16LE + null terminator
        let wide: &[u16] = std::slice::from_raw_parts(
            buf.as_ptr() as *const u16,
            buf_size as usize / 2,
        );
        let s = String::from_utf16_lossy(wide)
            .trim_end_matches('\0')
            .to_string();
        Ok(s)
    }
}

fn write_value(exe_path: &str) -> Result<()> {
    unsafe {
        let subkey = HSTRING::from(SUBKEY);
        let mut hkey = HKEY::default();
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &subkey,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut hkey,
            None,
        );
        if status != ERROR_SUCCESS {
            anyhow::bail!("RegCreateKeyExW (write) failed: {:?}", status);
        }

        let name = HSTRING::from(VALUE_NAME);
        // UTF-16LE エンコード + null terminator
        let wide: Vec<u16> = exe_path.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes: &[u8] = std::slice::from_raw_parts(
            wide.as_ptr() as *const u8,
            wide.len() * 2,
        );

        let status = RegSetValueExW(
            hkey,
            &name,
            0,
            REG_SZ,
            Some(bytes),
        );
        let _ = RegCloseKey(hkey);

        if status != ERROR_SUCCESS {
            anyhow::bail!("RegSetValueExW failed: {:?}", status);
        }
    }
    Ok(())
}