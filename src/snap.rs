/// スナップ計算：ウィンドウの現在 RECT をグリッドに丸める。
/// ドラッグ方向（WM_SIZING の wParam）を考慮し、動かしていない辺は保持する。

use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS},
    UI::Input::KeyboardAndMouse::GetAsyncKeyState,
    UI::WindowsAndMessaging::{
        GetWindowRect,
        SetWindowPos,
        SWP_NOZORDER, SWP_NOACTIVATE,
        WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT,
        WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
    },
};

use crate::grid::Grid;

/// DWM 不可視ボーダー（ドロップシャドウ用拡張フレーム）のサイズ。
/// Windows 10/11 では左右下に約 7px の不可視フレームが付く。
#[derive(Debug, Clone, Copy)]
pub struct DwmBorders {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl DwmBorders {
    pub fn zero() -> Self {
        Self { left: 0, top: 0, right: 0, bottom: 0 }
    }
}

/// GetWindowRect（不可視フレーム込み）と DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)
/// （可視領域）の差分から、各辺の不可視ボーダーサイズを返す。
/// DWM 呼び出しに失敗した場合は全辺 0 を返す。
pub fn get_dwm_borders(hwnd: HWND) -> DwmBorders {
    let outer = match get_window_rect(hwnd) {
        Some(r) => r,
        None => return DwmBorders::zero(),
    };
    let mut visible = RECT::default();
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr.is_err() {
        return DwmBorders::zero();
    }
    DwmBorders {
        left: visible.left - outer.left,
        top: visible.top - outer.top,
        right: outer.right - visible.right,
        bottom: outer.bottom - visible.bottom,
    }
}

/// **可視矩形** が (x, y, w, h) になるように SetWindowPos を呼ぶ。
/// DWM 不可視ボーダーを自動補償する。
/// 配置後に DWMWA_EXTENDED_FRAME_BOUNDS で実測し、ずれがあれば再補正する（2パス）。
/// auto_place.rs 等からも呼ばれる共通関数。
pub fn set_window_pos_visible(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    let b = get_dwm_borders(hwnd);
    let flags = SWP_NOZORDER | SWP_NOACTIVATE;

    // --- Pass 1: DWM ボーダー差分で補償して配置 ---
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x - b.left,
            y - b.top,
            w + b.left + b.right,
            h + b.top + b.bottom,
            flags,
        );
    }

    // --- Pass 2: 実測して誤差を補正 ---
    // DWMWA_EXTENDED_FRAME_BOUNDS と SetWindowPos の座標系の丸め誤差、
    // および DPI スケーリング環境での不一致を吸収する。
    if let Some(actual) = get_visible_rect(hwnd) {
        let dx = x - actual.left;
        let dy = y - actual.top;
        let dw = w - (actual.right - actual.left);
        let dh = h - (actual.bottom - actual.top);
        if dx != 0 || dy != 0 || dw != 0 || dh != 0 {
            eprintln!(
                "[GridSnap] set_window_pos_visible: correcting delta=({},{},{},{})",
                dx, dy, dw, dh
            );
            unsafe {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x - b.left + dx,
                    y - b.top + dy,
                    w + b.left + b.right + dw,
                    h + b.top + b.bottom + dh,
                    flags,
                );
            }
        }
    }
}

/// GetWindowRect の可視部分の矩形を返す（DWM ボーダーを除く）。
pub fn get_visible_rect(hwnd: HWND) -> Option<RECT> {
    let mut visible = RECT::default();
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut visible as *mut RECT as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if hr.is_ok() {
        Some(visible)
    } else {
        // DWM 失敗時は GetWindowRect にフォールバック
        get_window_rect(hwnd)
    }
}

/// EVENT_SYSTEM_MOVESIZEEND 後に呼ぶ。
/// Shift が押されていたらスナップをスキップする（微調整モード）。
/// drag_edge: WM_SIZING の wParam 相当。None の場合は移動スナップ（左上基準）。
pub fn apply_snap(hwnd: HWND, grid: &Grid, drag_edge: Option<u32>) {
    // Shift 押下 → スナップ解除
    if is_shift_pressed() {
        eprintln!("[GridSnap] apply_snap: Shift pressed, skipping snap");
        return;
    }

    // DWM 可視矩形を基準にスナップする（不可視ボーダーを除外）
    let vis = match get_visible_rect(hwnd) {
        Some(r) => r,
        None => {
            eprintln!("[GridSnap] apply_snap: get_visible_rect failed");
            return;
        }
    };
    eprintln!("[GridSnap] apply_snap: visible rect=({},{},{},{})",
        vis.left, vis.top, vis.right, vis.bottom);

    let (new_left, new_top, new_right, new_bottom) = match drag_edge {
        None => {
            // 移動スナップ: 可視左上を最寄りのグリッド交点に合わせる（F3）
            let snapped_left = grid.snap_x(vis.left);
            let snapped_top = grid.snap_y(vis.top);
            let vis_w = vis.right - vis.left;
            let vis_h = vis.bottom - vis.top;
            (snapped_left, snapped_top, snapped_left + vis_w, snapped_top + vis_h)
        }
        Some(edge) => {
            // リサイズスナップ: ドラッグした辺の可視境界を丸める（F2）
            let left   = if touches_left(edge)   { grid.snap_x(vis.left) }   else { vis.left };
            let top    = if touches_top(edge)     { grid.snap_y(vis.top) }    else { vis.top };
            let right  = if touches_right(edge)   { grid.snap_x(vis.right) }  else { vis.right };
            let bottom = if touches_bottom(edge)  { grid.snap_y(vis.bottom) } else { vis.bottom };
            (left, top, right, bottom)
        }
    };

    eprintln!("[GridSnap] apply_snap: snapped visible rect=({},{},{},{})",
        new_left, new_top, new_right, new_bottom);

    let vis_w = new_right - new_left;
    let vis_h = new_bottom - new_top;
    if vis_w <= 0 || vis_h <= 0 {
        eprintln!("[GridSnap] apply_snap: degenerate rect, skipping");
        return;
    }

    // set_window_pos_visible が DWM ボーダーを自動補償する
    set_window_pos_visible(hwnd, new_left, new_top, vis_w, vis_h);
    eprintln!("[GridSnap] apply_snap: done");
}

fn get_window_rect(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let mut rect = RECT::default();
        if GetWindowRect(hwnd, &mut rect).is_ok() {
            Some(rect)
        } else {
            None
        }
    }
}

fn is_shift_pressed() -> bool {
    unsafe {
        // 最上位ビットが立っていれば押下中
        (GetAsyncKeyState(0x10 /* VK_SHIFT */) as u16) & 0x8000 != 0
    }
}

// WM_SIZING wParam ヘルパー
// WMSZ_* は u32 定数なので matches! は使えない。== で比較する。
fn touches_left(edge: u32) -> bool {
    edge == WMSZ_LEFT || edge == WMSZ_TOPLEFT || edge == WMSZ_BOTTOMLEFT
}
fn touches_right(edge: u32) -> bool {
    edge == WMSZ_RIGHT || edge == WMSZ_TOPRIGHT || edge == WMSZ_BOTTOMRIGHT
}
fn touches_top(edge: u32) -> bool {
    edge == WMSZ_TOP || edge == WMSZ_TOPLEFT || edge == WMSZ_TOPRIGHT
}
fn touches_bottom(edge: u32) -> bool {
    edge == WMSZ_BOTTOM || edge == WMSZ_BOTTOMLEFT || edge == WMSZ_BOTTOMRIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_top_cases() {
        assert!(touches_top(WMSZ_TOP));
        assert!(touches_top(WMSZ_TOPLEFT));
        assert!(touches_top(WMSZ_TOPRIGHT));
        assert!(!touches_top(WMSZ_BOTTOM));
        assert!(!touches_top(WMSZ_LEFT));
        assert!(!touches_top(WMSZ_RIGHT));
    }

    #[test]
    fn touches_bottom_cases() {
        assert!(touches_bottom(WMSZ_BOTTOM));
        assert!(touches_bottom(WMSZ_BOTTOMLEFT));
        assert!(touches_bottom(WMSZ_BOTTOMRIGHT));
        assert!(!touches_bottom(WMSZ_TOP));
        assert!(!touches_bottom(WMSZ_LEFT));
    }

    #[test]
    fn touches_left_cases() {
        assert!(touches_left(WMSZ_LEFT));
        assert!(touches_left(WMSZ_TOPLEFT));
        assert!(touches_left(WMSZ_BOTTOMLEFT));
        assert!(!touches_left(WMSZ_RIGHT));
        assert!(!touches_left(WMSZ_TOP));
    }

    #[test]
    fn touches_right_cases() {
        assert!(touches_right(WMSZ_RIGHT));
        assert!(touches_right(WMSZ_TOPRIGHT));
        assert!(touches_right(WMSZ_BOTTOMRIGHT));
        assert!(!touches_right(WMSZ_LEFT));
    }

    #[test]
    fn corner_touches_two_edges() {
        assert!(touches_left(WMSZ_TOPLEFT) && touches_top(WMSZ_TOPLEFT));
        assert!(touches_right(WMSZ_TOPRIGHT) && touches_top(WMSZ_TOPRIGHT));
        assert!(touches_left(WMSZ_BOTTOMLEFT) && touches_bottom(WMSZ_BOTTOMLEFT));
        assert!(touches_right(WMSZ_BOTTOMRIGHT) && touches_bottom(WMSZ_BOTTOMRIGHT));
    }

    #[test]
    fn edge_touches_exactly_one_axis() {
        assert!(touches_left(WMSZ_LEFT) && !touches_top(WMSZ_LEFT) && !touches_bottom(WMSZ_LEFT));
        assert!(touches_right(WMSZ_RIGHT) && !touches_top(WMSZ_RIGHT) && !touches_bottom(WMSZ_RIGHT));
        assert!(touches_top(WMSZ_TOP) && !touches_left(WMSZ_TOP) && !touches_right(WMSZ_TOP));
        assert!(touches_bottom(WMSZ_BOTTOM) && !touches_left(WMSZ_BOTTOM) && !touches_right(WMSZ_BOTTOM));
    }
}