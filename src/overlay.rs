/// F4 グリッドオーバーレイ描画。
/// ドラッグ開始時に全モニターにグリッド線を半透明で描画し、ドロップで消える。
/// DLL 不要: 透過レイヤードウィンドウ + GDI で実装する。

use anyhow::Result;
use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreatePen, DeleteObject, EndPaint, InvalidateRect,
        LineTo, MoveToEx, PS_SOLID, SelectObject, PAINTSTRUCT,
        HDC,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics,
        RegisterClassExW, ShowWindow,
        CS_HREDRAW, CS_VREDRAW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        SW_HIDE, SW_SHOWNOACTIVATE,
        WM_PAINT, WNDCLASSEXW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
        WS_POPUP,
    },
    System::LibraryLoader::GetModuleHandleW,
};
use windows_core::PCWSTR;

use crate::{
    config::Config,
    monitor::enumerate_monitors,
};

const OVERLAY_CLASS: &str = "GridSnapOverlay";

/// 透過オーバーレイウィンドウ
pub struct OverlayWindow {
    hwnd: HWND,
}

// HWND は生ポインタだが、このスレッド専用のため Send/Sync を手動実装
unsafe impl Send for OverlayWindow {}
unsafe impl Sync for OverlayWindow {}

impl OverlayWindow {
    /// オーバーレイウィンドウを作成する（初期状態は非表示）。
    pub fn new() -> Result<Self> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            // ウィンドウクラス登録
            let class_name: Vec<u16> = OVERLAY_CLASS
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(overlay_wnd_proc),
                hInstance: hinstance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            // RegisterClassExW は重複登録エラーを無視する
            RegisterClassExW(&wc);

            // 仮想スクリーン全体を覆うウィンドウを作成
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                x, y, w, h,
                None, None, hinstance, None,
            )?;

            Ok(Self { hwnd })
        }
    }

    /// グリッドオーバーレイを表示する。
    pub fn show(&self, config: &Config) {
        let _ = config;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = InvalidateRect(self.hwnd, None, true);
        }
    }

    /// オーバーレイを非表示にする。
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// オーバーレイウィンドウのプロシージャ。
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_PAINT {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        draw_grid_lines(hdc);
        EndPaint(hwnd, &ps);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 全モニターのグリッド線を描画する。
fn draw_grid_lines(hdc: HDC) {
    let default_config = Config::default();

    let monitors = match enumerate_monitors() {
        Ok(m) => m,
        Err(_) => return,
    };

    unsafe {
        let pen = CreatePen(PS_SOLID, 1, COLORREF(0x00FF8000));
        let old_pen = SelectObject(hdc, pen);

        for monitor in &monitors {
            let grid = monitor.to_grid(&default_config);
            let lines = grid.grid_lines();

            for &x in &lines.verticals {
                MoveToEx(hdc, x, lines.origin_y, None);
                LineTo(hdc, x, lines.origin_y + lines.height);
            }
            for &y in &lines.horizontals {
                MoveToEx(hdc, lines.origin_x, y, None);
                LineTo(hdc, lines.origin_x + lines.width, y);
            }
        }

        SelectObject(hdc, old_pen);
        DeleteObject(pen);

        // --- 中央線を赤で描画 ---
        let red_pen = CreatePen(PS_SOLID, 2, COLORREF(0x000000FF));
        let old_pen2 = SelectObject(hdc, red_pen);

        for monitor in &monitors {
            let grid = monitor.to_grid(&default_config);
            let lines = grid.grid_lines();

            // 垂直中央線
            let mid_col = grid.columns as usize / 2;
            if mid_col < lines.verticals.len() {
                let x = lines.verticals[mid_col];
                MoveToEx(hdc, x, lines.origin_y, None);
                LineTo(hdc, x, lines.origin_y + lines.height);
            }

            // 水平中央線
            let mid_row = grid.rows as usize / 2;
            if mid_row < lines.horizontals.len() {
                let y = lines.horizontals[mid_row];
                MoveToEx(hdc, lines.origin_x, y, None);
                LineTo(hdc, lines.origin_x + lines.width, y);
            }
        }

        SelectObject(hdc, old_pen2);
        DeleteObject(red_pen);
    }
}