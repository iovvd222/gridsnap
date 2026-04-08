#![allow(warnings)]

use anyhow::Result;
use gridsnap::config::Config;

#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use gridsnap::event_hook::EventHookManager;

fn main() -> Result<()> {
    // Per-Monitor DPI Aware V2: 全 Win32 API が物理ピクセル座標を返すようにする
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::UI::HiDpi::{
            SetProcessDpiAwarenessContext,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        };
        let _ = SetProcessDpiAwarenessContext(
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    env_logger::init();

    let config = Config::load()?;
    eprintln!(
        "[GridSnap] Started. Grid: {}x{}",
        config.grid.columns, config.grid.rows,
    );

    // スタートアップ自動登録
    #[cfg(target_os = "windows")]
    if let Err(e) = gridsnap::startup::ensure_registered() {
        log::warn!("Failed to register startup: {:#}", e);
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = gridsnap::mac_startup::ensure_registered() {
        log::warn!("Failed to register startup: {:#}", e);
    }

    #[cfg(target_os = "windows")]
    {
        let config = Arc::new(Mutex::new(config));

        // イベントフックを起動し、メッセージループで待機する
        let _hook_manager = EventHookManager::new(Arc::clone(&config))?;

        // システムトレイを設置（Columns/Rows 変更 + Capture Position）
        let config_for_tray = config.lock().unwrap().clone();
        let _tray_hwnd = gridsnap::tray::setup(&config_for_tray);

        // Win32 メッセージループ
        // SetWinEventHook はフックを登録したスレッドのメッセージループが必要
        message_loop();
    }

    #[cfg(target_os = "macos")]
    {
        // AXObserver + CFRunLoop でイベント駆動
        let config_clone = config.clone();
        let _hook_manager = gridsnap::mac_event_hook::EventHookManager::new(config)?;
        // NSApplication 初期化後、run の前にトレイを設置
        eprintln!("[GridSnap] About to call mac_tray::setup...");
        gridsnap::mac_tray::setup(&config_clone);
        eprintln!("[GridSnap] mac_tray::setup returned OK");
        gridsnap::mac_event_hook::run_loop();
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        anyhow::bail!("Unsupported platform");
    }

    Ok(())
}

/// Win32 メッセージループ。
/// WM_QUIT を受け取るまでブロックする。
/// スレッドメッセージ（hwnd == 0）のカスタムメッセージもここで処理する。
#[cfg(target_os = "windows")]
fn message_loop() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, TranslateMessage, DispatchMessageW, MSG};
    use gridsnap::auto_place::{WM_GRIDSNAP_AUTO_PLACE, handle_deferred_auto_place};

    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // スレッドメッセージ（PostMessageW(None, ...) で投函されたもの）
            // は DispatchMessageW では配送されないため、ここで直接処理する。
            if msg.message == WM_GRIDSNAP_AUTO_PLACE {
                let target_hwnd = HWND(msg.wParam.0 as *mut _);
                handle_deferred_auto_place(target_hwnd);
                continue;
            }

            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}