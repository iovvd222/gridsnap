/// F0 新規ウィンドウ自動配置。
/// EVENT_OBJECT_SHOW 直後に呼ばれる。
/// - app_rules にマッチ → ルール通り配置
/// - マッチなし → アクティブウィンドウの右隣グリッドセルに配置（重なり許容）

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        GetForegroundWindow,
        GetWindowRect,
        GetClassNameW,
        IsWindowVisible,
        SetWindowPos,
        SWP_NOZORDER, SWP_NOACTIVATE,
    },
};

use crate::{
    config::{AppRule, Config},
    grid::Grid,
};

/// 自動配置を試みる。除外リストにマッチするか、不可視ウィンドウなら何もしない。
/// `device_name` はウィンドウが属するモニターの識別子（Windows: szDevice, macOS: display_id 文字列）。
pub fn try_auto_place(hwnd: HWND, grid: &Grid, config: &Config, device_name: &str) {
    if !is_target_window(hwnd, config) {
        return;
    }

    // app_rules マッチ（モニター別フィルタ付き）
    let class_name = get_class_name(hwnd);
    if let Some(rule) = find_matching_rule(&class_name, hwnd, config, device_name) {
        apply_rule(hwnd, grid, rule);
        return;
    }

    // フォールバック: アクティブウィンドウの右隣セルに配置
    place_next_to_active(hwnd, grid);
}

/// 自動配置の対象か判定する。
fn is_target_window(hwnd: HWND, config: &Config) -> bool {
    unsafe {
        // 不可視ウィンドウは対象外
        if !IsWindowVisible(hwnd).as_bool() {
            return false;
        }
    }

    // 除外クラス名チェック
    let class_name = get_class_name(hwnd);
    for excluded in &config.auto_place_exclude {
        if class_name.contains(excluded.as_str()) {
            return false;
        }
    }

    // 最小サイズフィルタ（タスクバーボタン等を除外）
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
/// モニター固有ルール（monitor = Some）を先に評価し、共通ルール（monitor = None）をフォールバックにする。
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
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// アクティブウィンドウの右隣グリッドセルに新規ウィンドウを配置する。
/// 右端を超えた場合は col=0 に折り返す（重なり許容）。
fn place_next_to_active(hwnd: HWND, grid: &Grid) {
    let active_right = unsafe {
        let active = GetForegroundWindow();
        if active.is_invalid() || active == hwnd {
            return;
        }
        let mut rect = RECT::default();
        if GetWindowRect(active, &mut rect).is_err() {
            return;
        }
        rect.right
    };

    // アクティブウィンドウの右端が属するグリッド列を特定し、その右の列に配置
    let cw = grid.cell_width();
    if cw <= 0 {
        return;
    }
    let relative_right = active_right - grid.origin_x;
    let right_col = (relative_right + cw - 1) / cw; // 切り上げ
    let next_col = right_col.min(grid.columns - 1);

    let rect = grid.cell_rect(next_col as u32, 0, 1, grid.rows as u32);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
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
        let len = GetModuleFileNameExW(handle, None, &mut buf);
        let full = String::from_utf16_lossy(&buf[..len as usize]);
        // ベース名だけを返す
        full.split(['\\', '/']).last().unwrap_or("").to_string()
    }
}