//! macOS ウィンドウスナップ (Accessibility API).
//! Windows の snap.rs に対応する macOS 実装。

use crate::grid::Grid;
use crate::mac_ffi::*;
use std::os::raw::c_void;

/// ウィンドウの位置 (x, y) をスクリーン座標で取得する。
pub fn get_window_position(window: AXUIElementRef) -> Option<(f64, f64)> {
    let attr = cf_str("AXPosition");
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(window, attr, &mut value)
    };
    unsafe { CFRelease(attr) };
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    let mut point = CGPoint::default();
    let ok = unsafe {
        AXValueGetValue(
            value,
            kAXValueTypeCGPoint,
            &mut point as *mut _ as *mut c_void,
        )
    };
    unsafe { CFRelease(value) };
    if ok != 0 {
        Some((point.x, point.y))
    } else {
        None
    }
}

/// ウィンドウのサイズ (width, height) を取得する。
pub fn get_window_size(window: AXUIElementRef) -> Option<(f64, f64)> {
    let attr = cf_str("AXSize");
    let mut value: CFTypeRef = std::ptr::null();
    let err = unsafe {
        AXUIElementCopyAttributeValue(window, attr, &mut value)
    };
    unsafe { CFRelease(attr) };
    if err != kAXErrorSuccess || value.is_null() {
        return None;
    }
    let mut size = CGSize::default();
    let ok = unsafe {
        AXValueGetValue(
            value,
            kAXValueTypeCGSize,
            &mut size as *mut _ as *mut c_void,
        )
    };
    unsafe { CFRelease(value) };
    if ok != 0 {
        Some((size.width, size.height))
    } else {
        None
    }
}

/// ウィンドウの位置を設定する。
pub fn set_window_position(window: AXUIElementRef, x: f64, y: f64) -> bool {
    let point = CGPoint { x, y };
    let value = unsafe {
        AXValueCreate(
            kAXValueTypeCGPoint,
            &point as *const _ as *const c_void,
        )
    };
    if value.is_null() {
        return false;
    }
    let attr = cf_str("AXPosition");
    let err = unsafe {
        AXUIElementSetAttributeValue(window, attr, value)
    };
    unsafe {
        CFRelease(attr);
        CFRelease(value);
    };
    err == kAXErrorSuccess
}

/// ウィンドウのサイズを設定する。
pub fn set_window_size(window: AXUIElementRef, w: f64, h: f64) -> bool {
    let size = CGSize { width: w, height: h };
    let value = unsafe {
        AXValueCreate(
            kAXValueTypeCGSize,
            &size as *const _ as *const c_void,
        )
    };
    if value.is_null() {
        return false;
    }
    let attr = cf_str("AXSize");
    let err = unsafe {
        AXUIElementSetAttributeValue(window, attr, value)
    };
    unsafe {
        CFRelease(attr);
        CFRelease(value);
    };
    err == kAXErrorSuccess
}

/// Shift キーが押されているか判定する。
pub fn is_shift_pressed() -> bool {
    unsafe {
        CGEventSourceKeyState(kCGEventSourceStateCombinedSessionState, kVK_Shift)
            || CGEventSourceKeyState(
                kCGEventSourceStateCombinedSessionState,
                kVK_RightShift,
            )
    }
}

/// マウス左ボタンが押下中か判定する。
pub fn is_mouse_down() -> bool {
    // 確実なマウス状態判定のため、CoreGraphicsのグローバルイベント状態を使用
    unsafe {
        crate::mac_ffi::CGEventSourceButtonState(
            crate::mac_ffi::kCGEventSourceStateCombinedSessionState,
            0,
        )
    }
}

/// ウィンドウにスナップを適用する（中心点基準）。
/// ウィンドウの中心点がどのグリッドセルにあるかを判定し、
/// 現在のサイズに最も近いセル数にフィットさせる。
/// メニューバー領域への配置を防ぐため、四辺独立スナップではなくセル単位で動く。
pub fn apply_snap(window: AXUIElementRef, grid: &Grid) {
    if is_shift_pressed() {
        eprintln!("[GridSnap] apply_snap: Shift pressed, skipping");
        return;
    }

    let (x, y) = match get_window_position(window) {
        Some(pos) => pos,
        None => {
            eprintln!("[GridSnap] apply_snap: failed to get position");
            return;
        }
    };
    let (w, h) = match get_window_size(window) {
        Some(sz) => sz,
        None => {
            eprintln!("[GridSnap] apply_snap: failed to get size");
            return;
        }
    };

    eprintln!(
        "[GridSnap] apply_snap: pos=({:.0},{:.0}) size=({:.0},{:.0})",
        x, y, w, h
    );

    let cell_w = grid.cell_width() as f64;
    let cell_h = grid.cell_height() as f64;

    // ウィンドウが何セル分にまたがるか（最低1セル）
    let col_span = (w / cell_w).round().max(1.0) as i32;
    let row_span = (h / cell_h).round().max(1.0) as i32;

    // ウィンドウ中心点
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;

    // 中心点がどのセル列・行にあるか（実数）
    let center_col_f = (cx - grid.origin_x as f64) / cell_w;
    let center_row_f = (cy - grid.origin_y as f64) / cell_h;

    // スパンの開始セルを中心点から算出し、グリッド範囲にクランプ
    let start_col = (center_col_f - col_span as f64 / 2.0).round() as i32;
    let start_row = (center_row_f - row_span as f64 / 2.0).round() as i32;
    let start_col = start_col.clamp(0, (grid.columns - col_span).max(0));
    let start_row = start_row.clamp(0, (grid.rows - row_span).max(0));

    let snapped_left = grid.col_to_x(start_col) as f64;
    let snapped_top = grid.row_to_y(start_row) as f64;
    let new_w = (col_span * grid.cell_width()) as f64;
    let new_h = (row_span * grid.cell_height()) as f64;

    eprintln!(
        "[GridSnap] apply_snap: center=({:.0},{:.0}) span={}x{} → cell({},{}) pos=({:.0},{:.0}) size=({:.0},{:.0})",
        cx, cy, col_span, row_span, start_col, start_row, snapped_left, snapped_top, new_w, new_h
    );

    let pos_changed = (snapped_left - x).abs() > 0.5 || (snapped_top - y).abs() > 0.5;
    let size_changed = (new_w - w).abs() > 0.5 || (new_h - h).abs() > 0.5;

    if size_changed {
        let ok = set_window_size(window, new_w, new_h);
        eprintln!("[GridSnap] apply_snap: set_size={}", ok);
    }
    if pos_changed {
        let ok = set_window_position(window, snapped_left, snapped_top);
        eprintln!("[GridSnap] apply_snap: set_position={}", ok);
    }
    if !pos_changed && !size_changed {
        eprintln!("[GridSnap] apply_snap: already on grid, no change");
    }
}