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

/// ドラッグ前後の矩形差分から動いた辺を推定する。
/// 全辺が同程度にシフトしていれば移動（None）と判定する。
struct DragEdge {
    left: bool,
    right: bool,
    top: bool,
    bottom: bool,
}

fn infer_drag_edge(
    pre: (f64, f64, f64, f64),  // (x, y, w, h)
    post: (f64, f64, f64, f64),
) -> Option<DragEdge> {
    let dl = (post.0 - pre.0).abs();
    let dr = ((post.0 + post.2) - (pre.0 + pre.2)).abs();
    let dt = (post.1 - pre.1).abs();
    let db = ((post.1 + post.3) - (pre.1 + pre.3)).abs();

    const THRESH: f64 = 5.0;

    let left_moved = dl > THRESH;
    let right_moved = dr > THRESH;
    let top_moved = dt > THRESH;
    let bottom_moved = db > THRESH;

    // 移動判定: 左右同量 かつ 上下同量
    let is_move = (dl - dr).abs() <= THRESH
        && (dt - db).abs() <= THRESH
        && (left_moved || top_moved);

    if is_move {
        return None;
    }

    // どの辺も動いていない場合も移動扱い
    if !left_moved && !right_moved && !top_moved && !bottom_moved {
        return None;
    }

    Some(DragEdge {
        left: left_moved,
        right: right_moved,
        top: top_moved,
        bottom: bottom_moved,
    })
}

/// ウィンドウにスナップを適用する。
/// `pre_drag_rect` が Some の場合、ドラッグ前後の差分から辺を推定し、
/// リサイズならドラッグ辺のみスナップ、移動なら rect_to_cell → cell_rect で正規化する。
/// `pre_drag_rect` が None の場合（非ドラッグ操作・新規ウィンドウ）は全辺正規化。
pub fn apply_snap(
    window: AXUIElementRef,
    grid: &Grid,
    pre_drag_rect: Option<(f64, f64, f64, f64)>,
) {
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

    let edge = pre_drag_rect.and_then(|pre| infer_drag_edge(pre, (x, y, w, h)));

    let (new_x, new_y, new_w, new_h) = match edge {
        Some(ref e) => {
            // リサイズ: ドラッグした辺のみ最寄りグリッド線にスナップ、固定辺は維持
            let left   = if e.left   { grid.snap_x(x as i32) as f64 }       else { x };
            let top    = if e.top    { grid.snap_y(y as i32) as f64 }        else { y };
            let right  = if e.right  { grid.snap_x((x + w) as i32) as f64 }  else { x + w };
            let bottom = if e.bottom { grid.snap_y((y + h) as i32) as f64 }   else { y + h };
            let sw = right - left;
            let sh = bottom - top;
            if sw <= 0.0 || sh <= 0.0 {
                eprintln!("[GridSnap] apply_snap: degenerate rect after resize snap, skipping");
                return;
            }
            eprintln!(
                "[GridSnap] apply_snap: resize snap edges(L={},R={},T={},B={})",
                e.left, e.right, e.top, e.bottom
            );
            (left, top, sw, sh)
        }
        None => {
            // 移動（または非ドラッグ操作）: rect_to_cell → cell_rect で全辺正規化
            let (col, row, cs, rs) = grid.rect_to_cell(x, y, w, h);
            let rect = grid.cell_rect(col, row, cs, rs);
            eprintln!(
                "[GridSnap] apply_snap: move snap → cell({},{}) span={}x{}",
                col, row, cs, rs
            );
            (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64)
        }
    };

    eprintln!(
        "[GridSnap] apply_snap: → pos=({:.0},{:.0}) size=({:.0},{:.0})",
        new_x, new_y, new_w, new_h
    );

    let pos_changed = (new_x - x).abs() > 0.5 || (new_y - y).abs() > 0.5;
    let size_changed = (new_w - w).abs() > 0.5 || (new_h - h).abs() > 0.5;

    if size_changed {
        let ok = set_window_size(window, new_w, new_h);
        eprintln!("[GridSnap] apply_snap: set_size={}", ok);
    }
    if pos_changed {
        let ok = set_window_position(window, new_x, new_y);
        eprintln!("[GridSnap] apply_snap: set_position={}", ok);
    }
    if !pos_changed && !size_changed {
        eprintln!("[GridSnap] apply_snap: already on grid, no change");
    }
}