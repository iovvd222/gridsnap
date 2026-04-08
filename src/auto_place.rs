/// F0 新規ウィンドウ自動配置。
/// EVENT_OBJECT_SHOW 直後に呼ばれる。
/// - app_rules にマッチ → ルール通り配置
/// - マッチなし → 何もしない（ドラッグ/リサイズ時にスナップで正規化される）
///
/// v2: コールバック内で即座に SetWindowPos せず、PostMessageW で
///     メッセージループに委譲し、対象ウィンドウの描画初期化完了を
///     ポーリングしてから配置する（Excel リボン暗転対策）。

use std::sync::Mutex;
use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT, WPARAM},
    UI::WindowsAndMessaging::{
        GetWindowRect,
        GetClassNameW,
        GetWindowLongW,
        IsWindowVisible,
        PostMessageW,
        SendMessageTimeoutW,
        GWL_STYLE, GWL_EXSTYLE,
        WS_POPUP, WS_CAPTION,
        WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE,
        SMTO_ABORTIFHUNG, WM_NULL,
    },
};

use crate::{
    config::{AppRule, Config},
    grid::Grid,
    monitor::monitor_for_window,
    snap::set_window_pos_visible,
};

/// カスタムメッセージ: 自動配置の遅延実行
/// メッセージループ側で handle_deferred_auto_place() を呼ぶ。
pub const WM_GRIDSNAP_AUTO_PLACE: u32 = 0x8001; // WM_APP + 1

/// 遅延配置の対象ウィンドウキュー。
/// PostMessageW では WPARAM に HWND しか渡せないため、
/// ルール情報はここに退避する。
static DEFERRED_QUEUE: Mutex<Vec<DeferredPlacement>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct DeferredPlacement {
    hwnd: HWND,
    rule: AppRule,
    device_name: String,
}

// HWND は生ポインタだが、メッセージループと同一スレッドで消費するため安全
unsafe impl Send for DeferredPlacement {}
unsafe impl Sync for DeferredPlacement {}

/// 自動配置を試みる。
/// マッチするルールがあれば即座には配置せず、PostMessageW でメッセージループに委譲する。
pub fn try_auto_place(hwnd: HWND, grid: &Grid, config: &Config, device_name: &str) {
    if !is_target_window(hwnd, config) {
        return;
    }

    let class_name = get_class_name(hwnd);
    if let Some(rule) = find_matching_rule(&class_name, hwnd, config, device_name) {
        // ルール情報をキューに退避
        let deferred = DeferredPlacement {
            hwnd,
            rule: rule.clone(),
            device_name: device_name.to_string(),
        };
        DEFERRED_QUEUE.lock().unwrap().push(deferred);

        // メッセージループに配置を委譲
        // hwnd=None → スレッドメッセージとして投函（DispatchMessageW では届かないが
        // GetMessageW で取得可能）。main.rs 側で msg.hwnd == 0 のケースとして処理する。
        unsafe {
            let _ = PostMessageW(
                None,
                WM_GRIDSNAP_AUTO_PLACE,
                WPARAM(hwnd.0 as usize),
                LPARAM(0),
            );
        }
        return;
    }

    // ルール未登録: 何もしない
}

/// メッセージループから呼ばれる。
/// 対象ウィンドウが応答可能になるまでポーリングし、配置を実行する。
pub fn handle_deferred_auto_place(target_hwnd: HWND) {
    // キューから該当エントリを取り出す
    let entry = {
        let mut queue = DEFERRED_QUEUE.lock().unwrap();
        if let Some(pos) = queue.iter().position(|d| d.hwnd == target_hwnd) {
            queue.remove(pos)
        } else {
            eprintln!("[GridSnap] handle_deferred: no queued entry for {:?}", target_hwnd);
            return;
        }
    };

    // ウィンドウが描画可能になるまでポーリング（最大 500ms、50ms 間隔）
    const MAX_RETRIES: u32 = 10;
    const INTERVAL_MS: u64 = 50;

    for i in 0..MAX_RETRIES {
        if is_window_responsive(entry.hwnd) {
            eprintln!("[GridSnap] handle_deferred: window responsive after {}ms", i as u64 * INTERVAL_MS);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(INTERVAL_MS));
    }

    // Config を再取得してグリッド計算（ポーリング中に Config が変わる可能性は低いが安全側）
    let config_arc = match crate::event_hook::get_config() {
        Some(c) => c,
        None => {
            eprintln!("[GridSnap] handle_deferred: CONFIG is None");
            return;
        }
    };
    let config = config_arc.lock().unwrap();

    let monitor = match monitor_for_window(entry.hwnd) {
        Some(m) => m,
        None => {
            eprintln!("[GridSnap] handle_deferred: no monitor for window");
            return;
        }
    };
    let grid = monitor.to_grid(&config);

    apply_rule(entry.hwnd, &grid, &entry.rule);
    eprintln!("[GridSnap] handle_deferred: placed {:?}", entry.hwnd);
}

/// SendMessageTimeoutW(WM_NULL) でウィンドウが応答可能か判定する。
/// ウィンドウのメッセージループが初期化処理でブロックされている間は
/// タイムアウトし false を返す。
fn is_window_responsive(hwnd: HWND) -> bool {
    unsafe {
        let mut result = 0usize;
        let status = SendMessageTimeoutW(
            hwnd,
            WM_NULL,
            WPARAM(0),
            LPARAM(0),
            SMTO_ABORTIFHUNG,
            100, // 100ms タイムアウト
            Some(&mut result),
        );
        status.0 != 0
    }
}

/// 自動配置の対象か判定する。
fn is_target_window(hwnd: HWND, config: &Config) -> bool {
    unsafe {
        // 不可視ウィンドウは対象外
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
    }

    // ── ウィンドウスタイルによるフィルタ ──
    unsafe {
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

        // WS_EX_TOOLWINDOW: トレイ隠しウィンドウ、ツールチップ、フロート系
        if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
            return false;
        }

        // WS_EX_NOACTIVATE: 通知バナー、一部のポップアップ
        if (ex_style & WS_EX_NOACTIVATE.0) != 0 {
            return false;
        }

        // WS_POPUP かつキャプション（タイトルバー）なし
        // → メニュー (#32768)、ドロップダウン、ツールチップ、スプラッシュ等
        if (style & WS_POPUP.0) != 0 && (style & WS_CAPTION.0) == 0 {
            return false;
        }
    }

    // ── クラス名チェック ──
    let class_name = get_class_name(hwnd);

    const SYSTEM_EXCLUDE: &[&str] = &[
        "#32768",            // Win32 メニュー
        "tooltips_class32",  // ツールチップ
        "#32770",            // コモンダイアログ
    ];
    for &sys in SYSTEM_EXCLUDE {
        if class_name == sys {
            return false;
        }
    }

    // UWP / WinUI アプリは SetWindowPos で描画が壊れるため除外
    if class_name.contains("ApplicationFrameWindow")
        || class_name.contains("Windows.UI.Core.CoreWindow")
    {
        return false;
    }

    for excluded in &config.auto_place_exclude {
        if class_name.contains(excluded.as_str()) {
            return false;
        }
    }

    // 最小サイズフィルタ
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            if w < 100 || h < 100 {
                return false;
            }
        }
    }

    true
}

/// app_rules から最初にマッチするルールを返す。
fn find_matching_rule<'a>(
    class_name: &str,
    hwnd: HWND,
    config: &'a Config,
    device_name: &str,
) -> Option<&'a AppRule> {
    let exe_name = get_exe_name(hwnd);

    let matches_app = |rule: &AppRule| -> bool {
        let class_match = rule
            .class_name
            .as_ref()
            .map(|c| class_name.contains(c.as_str()))
            .unwrap_or(true);
        let exe_match = rule
            .exe_name
            .as_ref()
            .map(|e| exe_name.contains(e.as_str()))
            .unwrap_or(true);
        class_match && exe_match
    };

    // Pass 1: モニター固有ルール（部分一致）
    for rule in &config.app_rules {
        if let Some(ref mon) = rule.monitor {
            if device_name.contains(mon.as_str()) && matches_app(rule) {
                return Some(rule);
            }
        }
    }

    // Pass 2: 共通ルール（monitor = None）
    for rule in &config.app_rules {
        if rule.monitor.is_none() && matches_app(rule) {
            return Some(rule);
        }
    }

    None
}

/// ルール通りにウィンドウを配置する。
fn apply_rule(hwnd: HWND, grid: &Grid, rule: &AppRule) {
    let rect = grid.cell_rect(rule.col, rule.row, rule.col_span, rule.row_span);
    set_window_pos_visible(hwnd, rect.x, rect.y, rect.w, rect.h);
}

/// HWND のウィンドウクラス名を取得する。
fn get_class_name(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let len = GetClassNameW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// HWND のプロセス実行ファイル名を取得する（ベース名のみ）。
fn get_exe_name(hwnd: HWND) -> String {
    use windows::Win32::{
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        UI::WindowsAndMessaging::GetWindowThreadProcessId,
        System::ProcessStatus::GetModuleFileNameExW,
    };
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let handle = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(h) => h,
            Err(_) => return String::new(),
        };
        let mut buf = [0u16; 260];
        let len = GetModuleFileNameExW(Some(handle), None, &mut buf);
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        full.split(['\\', '/']).last().unwrap_or("").to_string()
    }
}