/// SetWinEventHook による EVENT_SYSTEM_MOVESIZEEND / EVENT_OBJECT_SHOW の購読。
/// フックコールバックはメッセージループと同一スレッドで呼ばれる。

use anyhow::{Context, Result};
use std::sync::{Arc, Mutex};
use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
    UI::WindowsAndMessaging::{
        GetWindowRect,
        EVENT_OBJECT_SHOW, EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZESTART,
        WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
        WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_BOTTOM,
        WMSZ_TOPLEFT, WMSZ_TOPRIGHT, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT,
    },
};

#[cfg(target_os = "macos")]
use crate::mac_ffi;
#[cfg(target_os = "macos")]
use crate::mac_monitor;
#[cfg(target_os = "macos")]
use crate::mac_snap;
#[cfg(target_os = "macos")]
use objc;

use crate::{
    auto_place,
    config::Config,
    monitor::monitor_for_window,
    overlay::OverlayWindow,
    snap::apply_snap,
};

/// フックハンドルをまとめて管理し、Drop 時に解除する。
pub struct EventHookManager {
    hooks: Vec<HWINEVENTHOOK>,
}

impl Drop for EventHookManager {
    fn drop(&mut self) {
        unsafe {
            for h in &self.hooks {
                let _ = UnhookWinEvent(*h);
            }
        }
    }
}

// コールバックからアクセスする global state
// メッセージループと同スレッドなので Mutex で十分
static CONFIG: Mutex<Option<Arc<Mutex<Config>>>> = Mutex::new(None);
static OVERLAY: Mutex<Option<Arc<Mutex<OverlayWindow>>>> = Mutex::new(None);
/// ドラッグ開始時のウィンドウ矩形（辺推定用）
static PRE_DRAG_RECT: Mutex<Option<RECT>> = Mutex::new(None);

impl EventHookManager {
    pub fn new(config: Arc<Mutex<Config>>) -> Result<Self> {
        // global state に格納
        *CONFIG.lock().unwrap() = Some(Arc::clone(&config));

        // オーバーレイウィンドウを作成
        let overlay = Arc::new(Mutex::new(OverlayWindow::new()?));
        *OVERLAY.lock().unwrap() = Some(Arc::clone(&overlay));

        let mut hooks = Vec::new();

        unsafe {
            // F2/F3: リサイズ・移動完了
            let h1 = SetWinEventHook(
                EVENT_SYSTEM_MOVESIZEEND,
                EVENT_SYSTEM_MOVESIZEEND,
                None,
                Some(on_move_size_end),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if h1.is_invalid() {
                anyhow::bail!("SetWinEventHook(EVENT_SYSTEM_MOVESIZEEND) failed");
            }
            hooks.push(h1);

            // F4: ドラッグ開始 → オーバーレイ表示
            let h_start = SetWinEventHook(
                EVENT_SYSTEM_MOVESIZESTART,
                EVENT_SYSTEM_MOVESIZESTART,
                None,
                Some(on_move_size_start),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if h_start.is_invalid() {
                anyhow::bail!("SetWinEventHook(EVENT_SYSTEM_MOVESIZESTART) failed");
            }
            hooks.push(h_start);

            // F0: 新規ウィンドウ表示
            let h2 = SetWinEventHook(
                EVENT_OBJECT_SHOW,
                EVENT_OBJECT_SHOW,
                None,
                Some(on_object_show),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            if h2.is_invalid() {
                anyhow::bail!("SetWinEventHook(EVENT_OBJECT_SHOW) failed");
            }
            hooks.push(h2);
        }

        Ok(Self { hooks })
    }
}

/// EVENT_SYSTEM_MOVESIZESTART コールバック。
/// ドラッグ開始時にオーバーレイを表示する。
unsafe extern "system" fn on_move_size_start(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _event_time: u32,
) {
    eprintln!("[GridSnap] on_move_size_start fired");

    // ドラッグ開始時の RECT を保存（辺推定用）
    let mut rect = RECT::default();
    if GetWindowRect(_hwnd, &mut rect).is_ok() {
        *PRE_DRAG_RECT.lock().unwrap() = Some(rect);
    }

    let config_arc = match CONFIG.lock().unwrap().clone() {
        Some(c) => c,
        None => return,
    };
    let config = config_arc.lock().unwrap();

    if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
        ov.lock().unwrap().show(&config);
    }
}

/// EVENT_SYSTEM_MOVESIZEEND コールバック。
/// ドロップ直後に呼ばれるのでここでスナップを適用する。
unsafe extern "system" fn on_move_size_end(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _event_time: u32,
) {
    eprintln!("[GridSnap] on_move_size_end fired: hwnd={:?}", hwnd);

    // オーバーレイを非表示にする（ドラッグ終了）
    if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
        ov.lock().unwrap().hide();
    }

    let config_arc = match CONFIG.lock().unwrap().clone() {
        Some(c) => c,
        None => {
            eprintln!("[GridSnap] on_move_size_end: CONFIG is None, returning");
            return;
        }
    };
    let config = config_arc.lock().unwrap();

    let monitor = match monitor_for_window(hwnd) {
        Some(m) => m,
        None => {
            eprintln!("[GridSnap] on_move_size_end: monitor_for_window returned None");
            return;
        }
    };
    let grid = monitor.to_grid(&config);
    eprintln!("[GridSnap] grid: {}x{}, cell={}x{}, origin=({},{})",
        grid.columns, grid.rows, grid.cell_width(), grid.cell_height(),
        grid.origin_x, grid.origin_y);

    // ドラッグ開始時の RECT と比較してドラッグ辺を推定する
    let drag_edge = infer_drag_edge(hwnd);
    eprintln!("[GridSnap] inferred drag_edge: {:?}", drag_edge);
    apply_snap(hwnd, &grid, drag_edge);
}

/// ドラッグ前後の RECT を比較し、動いた辺から WMSZ_* を推定する。
/// 全辺が同程度に動いていれば移動（None）と判定する。
fn infer_drag_edge(hwnd: HWND) -> Option<u32> {
    let pre = match PRE_DRAG_RECT.lock().unwrap().take() {
        Some(r) => r,
        None => return None, // 開始時 RECT がなければ移動扱い
    };
    let mut post = RECT::default();
    unsafe {
        if GetWindowRect(hwnd, &mut post).is_err() {
            return None;
        }
    }

    let dl = (post.left - pre.left).abs();
    let dr = (post.right - pre.right).abs();
    let dt = (post.top - pre.top).abs();
    let db = (post.bottom - pre.bottom).abs();

    const THRESH: i32 = 5; // ピクセル閾値

    let left_moved = dl > THRESH;
    let right_moved = dr > THRESH;
    let top_moved = dt > THRESH;
    let bottom_moved = db > THRESH;

    // 移動判定: 左右の移動量がほぼ同じ かつ 上下の移動量がほぼ同じ
    let is_move = (dl.abs_diff(dr) <= THRESH as u32) && (dt.abs_diff(db) <= THRESH as u32)
        && (left_moved || top_moved);

    if is_move {
        return None; // 移動 → 左上基準スナップ
    }

    // リサイズ: 動いた辺の組み合わせから WMSZ_* を返す
    match (left_moved, right_moved, top_moved, bottom_moved) {
        (true,  false, false, false) => Some(WMSZ_LEFT),
        (false, true,  false, false) => Some(WMSZ_RIGHT),
        (false, false, true,  false) => Some(WMSZ_TOP),
        (false, false, false, true)  => Some(WMSZ_BOTTOM),
        (true,  false, true,  false) => Some(WMSZ_TOPLEFT),
        (false, true,  true,  false) => Some(WMSZ_TOPRIGHT),
        (true,  false, false, true)  => Some(WMSZ_BOTTOMLEFT),
        (false, true,  false, true)  => Some(WMSZ_BOTTOMRIGHT),
        _ => None, // 判定不能 → 移動扱い
    }
}

// ──── Public: Config アクセサ ────

/// auto_place から Config を参照するための公開アクセサ。
pub fn get_config() -> Option<Arc<Mutex<Config>>> {
    CONFIG.lock().unwrap().clone()
}

// ──── Public: Config の動的更新 ────

/// 外部（tray）から Config を更新する。
/// クロージャ内で Config を変更し、変更後の Config を TOML に保存する。
pub fn update_config<F: FnOnce(&mut Config)>(f: F) {
    let config_arc = match CONFIG.lock().unwrap().clone() {
        Some(c) => c,
        None => return,
    };
    let mut config = config_arc.lock().unwrap();
    f(&mut config);
    if let Err(e) = config.save() {
        eprintln!("[GridSnap] Failed to save config: {:?}", e);
    }
    eprintln!(
        "[GridSnap] Config updated: {}x{}",
        config.grid.columns, config.grid.rows
    );
}

/// F0a: 最前面の他プロセスウィンドウの位置をキャプチャし、app_rules に upsert する。
/// トレイメニューの「Capture Position」から呼ばれる。
/// Zオーダーを走査し、自プロセスのウィンドウをスキップして最初の可視トップレベルウィンドウを返す。
/// 成功時はキャプチャ内容の文字列を返す。
pub fn capture_window(_target: Option<HWND>) -> Option<String> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowRect, GetClassNameW,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, GetCurrentProcessId, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    use windows::Win32::System::ProcessStatus::GetModuleFileNameExW;
    use windows::Win32::Foundation::RECT;

    unsafe {
        // Zオーダーを走査し、自プロセス以外の最前面可視ウィンドウを探す
        let my_pid = GetCurrentProcessId();
        let hwnd = find_topmost_foreign_window(my_pid);
        let hwnd = match hwnd {
            Some(h) => h,
            None => {
                eprintln!("[GridSnap] capture: no suitable target window found");
                return None;
            }
        };

        // exe 名を取得
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => {
                eprintln!("[GridSnap] capture: OpenProcess failed for pid {}", pid);
                return None;
            }
        };
        let mut buf = [0u16; 260];
        let len = GetModuleFileNameExW(Some(handle), None, &mut buf);
        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        let exe_name = full_path.split(['\\', '/']).last().unwrap_or("").to_string();
        if exe_name.is_empty() {
            eprintln!("[GridSnap] capture: could not get exe name for pid {}", pid);
            return None;
        }

        // ウィンドウの可視矩形を取得（DWM 不可視ボーダーを除外）
        let rect = match crate::snap::get_visible_rect(hwnd) {
            Some(r) => r,
            None => {
                eprintln!("[GridSnap] capture: get_visible_rect failed");
                return None;
            }
        };

        // モニター検出 -> グリッド -> セル座標に変換
        let monitor = match crate::monitor::monitor_for_window(hwnd) {
            Some(m) => m,
            None => {
                eprintln!("[GridSnap] capture: no monitor for window");
                return None;
            }
        };

        let config_arc = match CONFIG.lock().unwrap().clone() {
            Some(c) => c,
            None => {
                eprintln!("[GridSnap] capture: CONFIG not initialized");
                return None;
            }
        };
        let mut config = config_arc.lock().unwrap();
        let grid = monitor.to_grid(&config);

        let (col, row, col_span, row_span) = grid.rect_to_cell(
            rect.left as f64,
            rect.top as f64,
            (rect.right - rect.left) as f64,
            (rect.bottom - rect.top) as f64,
        );

        // app_rules に upsert -> TOML 保存
        let rule = crate::config::AppRule {
            monitor: Some(monitor.device_name.clone()),
            class_name: None,
            exe_name: Some(exe_name.clone()),
            col,
            row,
            col_span,
            row_span,
        };
        config.upsert_app_rule(rule);
        if let Err(e) = config.save() {
            eprintln!("[GridSnap] capture: Failed to save config: {:?}", e);
        }

        let msg = format!(
            "Captured: {} [{}] -> ({},{}) {}x{}",
            exe_name, monitor.device_name, col, row, col_span, row_span
        );
        eprintln!("[GridSnap] {}", msg);
        Some(msg)
    }
}

/// EVENT_OBJECT_SHOW コールバック。
/// 新規ウィンドウを検出し、F0 自動配置を適用する。
unsafe extern "system" fn on_object_show(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _id_event_thread: u32,
    _event_time: u32,
) {
    // OBJID_WINDOW (0) のみ対象
    if id_object != 0 {
        return;
    }

    // EVENT_OBJECT_SHOW ではオーバーレイを操作しない（F4 は MOVESIZESTART で処理）

    let config_arc = match CONFIG.lock().unwrap().clone() {
        Some(c) => c,
        None => return,
    };
    let config = config_arc.lock().unwrap();

    let monitor = match monitor_for_window(hwnd) {
        Some(m) => m,
        None => return,
    };
    let grid = monitor.to_grid(&config);

    auto_place::try_auto_place(hwnd, &grid, &config, &monitor.device_name);
}

/// Zオーダーを上から走査し、指定PID以外の最初の可視トップレベルウィンドウを返す。
/// タスクバーやツールウィンドウ等、通常のアプリウィンドウでないものはスキップする。
fn find_topmost_foreign_window(my_pid: u32) -> Option<HWND> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetDesktopWindow, IsWindowVisible,
        GetWindowLongW, GetWindowThreadProcessId,
        GW_CHILD, GW_HWNDNEXT, GWL_EXSTYLE, GWL_STYLE,
    };

    const WS_EX_TOOLWINDOW_RAW: i32 = 0x00000080;
    const WS_VISIBLE_RAW: i32 = 0x10000000;
    const WS_EX_NOACTIVATE_RAW: i32 = 0x08000000;

    unsafe {
        // デスクトップの最初の子 = Zオーダー最上位
        let desktop = GetDesktopWindow();
        let mut cur = GetWindow(desktop, GW_CHILD).ok();

        while let Some(hwnd) = cur {
            // 次へ進む準備
            let next = GetWindow(hwnd, GW_HWNDNEXT).ok();

            // 不可視ウィンドウをスキップ
            if !IsWindowVisible(hwnd).as_bool() {
                cur = next;
                continue;
            }

            // 自プロセスのウィンドウをスキップ
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == my_pid {
                cur = next;
                continue;
            }

            // ツールウィンドウ・NOACTIVATE をスキップ
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if (ex_style & WS_EX_TOOLWINDOW_RAW) != 0 {
                cur = next;
                continue;
            }
            if (ex_style & WS_EX_NOACTIVATE_RAW) != 0 {
                cur = next;
                continue;
            }

            // サイズが 0 のウィンドウをスキップ
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let w = rect.right - rect.left;
                let h = rect.bottom - rect.top;
                if w <= 0 || h <= 0 {
                    cur = next;
                    continue;
                }
            }

            return Some(hwnd);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::RECT;

    /// infer_drag_edge のコアロジックを RECT ペアから直接テストするヘルパー。
    /// 本体は HWND + global state 経由なので、ロジック部分だけ抽出。
    fn detect_edge(pre: &RECT, post: &RECT) -> Option<u32> {
        let dl = (post.left - pre.left).abs();
        let dr = (post.right - pre.right).abs();
        let dt = (post.top - pre.top).abs();
        let db = (post.bottom - pre.bottom).abs();
        const THRESH: i32 = 5;
        let left_moved = dl > THRESH;
        let right_moved = dr > THRESH;
        let top_moved = dt > THRESH;
        let bottom_moved = db > THRESH;
        let is_move = (dl.abs_diff(dr) <= THRESH as u32)
            && (dt.abs_diff(db) <= THRESH as u32)
            && (left_moved || top_moved);
        if is_move { return None; }
        match (left_moved, right_moved, top_moved, bottom_moved) {
            (true,  false, false, false) => Some(WMSZ_LEFT),
            (false, true,  false, false) => Some(WMSZ_RIGHT),
            (false, false, true,  false) => Some(WMSZ_TOP),
            (false, false, false, true)  => Some(WMSZ_BOTTOM),
            (true,  false, true,  false) => Some(WMSZ_TOPLEFT),
            (false, true,  true,  false) => Some(WMSZ_TOPRIGHT),
            (true,  false, false, true)  => Some(WMSZ_BOTTOMLEFT),
            (false, true,  false, true)  => Some(WMSZ_BOTTOMRIGHT),
            _ => None,
        }
    }

    #[test]
    fn infer_move_uniform_translation() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 200, top: 150, right: 600, bottom: 450 };
        assert_eq!(detect_edge(&pre, &post), None);
    }

    #[test]
    fn infer_resize_right_edge() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 100, top: 100, right: 600, bottom: 400 };
        assert_eq!(detect_edge(&pre, &post), Some(WMSZ_RIGHT));
    }

    #[test]
    fn infer_resize_left_edge() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 50, top: 100, right: 500, bottom: 400 };
        assert_eq!(detect_edge(&pre, &post), Some(WMSZ_LEFT));
    }

    #[test]
    fn infer_resize_bottom_right_corner() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 100, top: 100, right: 600, bottom: 500 };
        assert_eq!(detect_edge(&pre, &post), Some(WMSZ_BOTTOMRIGHT));
    }

    #[test]
    fn infer_resize_top_left_corner() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 50, top: 50, right: 500, bottom: 400 };
        assert_eq!(detect_edge(&pre, &post), Some(WMSZ_TOPLEFT));
    }

    #[test]
    fn infer_no_movement_returns_none() {
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 102, top: 101, right: 503, bottom: 402 };
        assert_eq!(detect_edge(&pre, &post), None);
    }

    #[test]
    fn infer_at_threshold_not_detected() {
        // 右辺がちょうど THRESH = 5 → 検出されない
        let pre = RECT { left: 100, top: 100, right: 500, bottom: 400 };
        let post = RECT { left: 100, top: 100, right: 505, bottom: 400 };
        assert_eq!(detect_edge(&pre, &post), None);
    }
}