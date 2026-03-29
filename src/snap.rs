/// スナップ計算：ウィンドウの現在 RECT をグリッドに丸める。
/// ドラッグ方向（WM_SIZING の wParam）を考慮し、動かしていない辺は保持する。

use windows::Win32::{
    Foundation::{HWND, RECT},
    UI::WindowsAndMessaging::{
        GetWindowRect,
        GetAsyncKeyState,
        SetWindowPos,
        SET_WINDOW_POS_FLAGS,
        SWP_NOZORDER, SWP_NOACTIVATE,
        WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_BOTTOMRIGHT,
        WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT,
    },
};

use crate::grid::Grid;

/// EVENT_SYSTEM_MOVESIZEEND 後に呼ぶ。
/// Shift が押されていたらスナップをスキップする（微調整モード）。
/// drag_edge: WM_SIZING の wParam 相当。None の場合は移動スナップ（左上基準）。
pub fn apply_snap(hwnd: HWND, grid: &Grid, drag_edge: Option<u32>) {
    // Shift 押下 → スナップ解除
    if is_shift_pressed() {
        return;
    }

    let rect = match get_window_rect(hwnd) {
        Some(r) => r,
        None => return,
    };

    let (new_left, new_top, new_right, new_bottom) = match drag_edge {
        None => {
            // 移動スナップ: 左上を最寄りのグリッド交点に合わせる（F3）
            let snapped_left = grid.snap_x(rect.left);
            let snapped_top = grid.snap_y(rect.top);
            let w = rect.right - rect.left;
            let h = rect.bottom - rect.top;
            (snapped_left, snapped_top, snapped_left + w, snapped_top + h)
        }
        Some(edge) => {
            // リサイズスナップ: ドラッグした辺のみ丸める（F2）
            let left = if touches_left(edge) { grid.snap_x(rect.left) } else { rect.left };
            let top = if touches_top(edge) { grid.snap_y(rect.top) } else { rect.top };
            let right = if touches_right(edge) { grid.snap_x(rect.right) } else { rect.right };
            let bottom = if touches_bottom(edge) { grid.snap_y(rect.bottom) } else { rect.bottom };
            (left, top, right, bottom)
        }
    };

    let w = new_right - new_left;
    let h = new_bottom - new_top;
    if w <= 0 || h <= 0 {
        return; // 縮退防止
    }

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            new_left,
            new_top,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
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
}