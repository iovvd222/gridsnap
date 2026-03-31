/// F4 グリッドオーバーレイ描画。
/// ドラッグ開始時に全モニターにグリッド線を半透明で描画し、ドロップで消える。
/// DLL 不要: 透過レイヤードウィンドウ + GDI で実装する。

use anyhow::Result;
use windows::Win32::{
    Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreatePen, DeleteObject, EndPaint, FillRect,
        GetStockObject, InvalidateRect,
        LineTo, MoveToEx, PS_SOLID, SelectObject, PAINTSTRUCT,
        BLACK_BRUSH, HDC, HBRUSH,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics,
        RegisterClassExW, SetLayeredWindowAttributes, ShowWindow,
        CS_HREDRAW, CS_VREDRAW, LWA_COLORKEY,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        SW_HIDE, SW_SHOWNOACTIVATE,
        WM_PAINT, WNDCLASSEXW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
        WS_POPUP,
    },
    System::LibraryLoader::GetModuleHandleW,
};
use windows_core::PCWSTR;

use std::sync::Mutex;

use crate::{
    config::Config,
    monitor::enumerate_monitors,
};

const OVERLAY_CLASS: &str = "GridSnapOverlay";

/// WM_PAINT コールバックから参照する Config（show() 時に更新される）
static OVERLAY_CONFIG: Mutex<Option<Config>> = Mutex::new(None);

/// 透過オーバーレイウィンドウ。
/// DWM 合成への干渉を避けるため、ウィンドウは show() 時に生成し hide() 時に破棄する。
pub struct OverlayWindow {
    hwnd: Option<HWND>,
    class_registered: bool,
    config: Option<Config>,  // WM_PAINT に渡すための Config キャッシュ
}

// HWND は生ポインタだが、このスレッド専用のため Send/Sync を手動実装
unsafe impl Send for OverlayWindow {}
unsafe impl Sync for OverlayWindow {}

impl OverlayWindow {
    /// OverlayWindow を初期化する。ウィンドウはまだ作らない。
    pub fn new() -> Result<Self> {
        Ok(Self {
            hwnd: None,
            class_registered: false,
            config: None,
        })
    }

    /// オーバーレイウィンドウを作成して返す。
    fn create_window(&mut self) -> Result<HWND> {
        unsafe {
            let hinstance = GetModuleHandleW(None)?;

            let class_name: Vec<u16> = OVERLAY_CLASS
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            if !self.class_registered {
                let wc = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(overlay_wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: PCWSTR(class_name.as_ptr()),
                    hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                    ..Default::default()
                };
                RegisterClassExW(&wc);
                self.class_registered = true;
            }

            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            // 端のグリッド線がウィンドウ境界でクリップされないよう、
            // 仮想スクリーンより少し大きく作る（余白は BLACK=透過色で不可視）
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN) + 2;
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN) + 2;

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                PCWSTR(class_name.as_ptr()),
                PCWSTR::null(),
                WS_POPUP,
                x, y, w, h,
                None, None, Some(hinstance.into()), None,
            )?;

            SetLayeredWindowAttributes(hwnd, COLORREF(0x00000000), 0, LWA_COLORKEY)?;

            Ok(hwnd)
        }
    }

    /// グリッドオーバーレイを表示する。ウィンドウがなければ生成する。
    pub fn show(&mut self, config: &Config) {
        self.config = Some(config.clone());
        // グローバルに Config を共有（WM_PAINT コールバックから参照するため）
        *OVERLAY_CONFIG.lock().unwrap() = Some(config.clone());
        if self.hwnd.is_none() {
            match self.create_window() {
                Ok(h) => self.hwnd = Some(h),
                Err(e) => {
                    eprintln!("[GridSnap] overlay create_window failed: {:?}", e);
                    return;
                }
            }
        }
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
        }
    }

    /// オーバーレイを非表示にし、ウィンドウを破棄する。
    /// 破棄後に全ウィンドウの再描画を強制し、UWP アプリの描画化けを防ぐ。
    pub fn hide(&mut self) {
        if let Some(hwnd) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(hwnd);
                // 全ウィンドウの再描画を強制（hwnd=None でデスクトップ全体）
                let _ = InvalidateRect(None, None, true);
            }
        }
    }
}

impl Drop for OverlayWindow {
    fn drop(&mut self) {
        self.hide();
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

        // レイヤードウィンドウでは WM_ERASEBKGND が信頼できないため、
        // 明示的に黒（透過色）で全面塗りつぶしてからグリッド線を描画
        FillRect(hdc, &ps.rcPaint, HBRUSH(GetStockObject(BLACK_BRUSH).0));

        draw_grid_lines(hdc);
        EndPaint(hwnd, &ps);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 全モニターのグリッド線を描画する。
/// スクリーン絶対座標 → クライアント座標への変換を行う。
fn draw_grid_lines(hdc: HDC) {
    let config = OVERLAY_CONFIG.lock().unwrap().clone()
        .unwrap_or_else(Config::default);

    let monitors = match enumerate_monitors() {
        Ok(m) => m,
        Err(_) => return,
    };

    // オーバーレイウィンドウの原点（仮想スクリーン左上）をオフセットとして取得
    // MoveToEx/LineTo はクライアント座標で動作するため、スクリーン座標からこの分を引く
    let offset_x = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let offset_y = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };

    unsafe {
        let pen = CreatePen(PS_SOLID, 1, COLORREF(0x00FF8000));
        let old_pen = SelectObject(hdc, pen.into());

        for monitor in &monitors {
            let grid = monitor.to_grid(&config);
            let lines = grid.grid_lines();

            for &x in &lines.verticals {
                MoveToEx(hdc, x - offset_x, lines.origin_y - offset_y, None);
                LineTo(hdc, x - offset_x, lines.origin_y + lines.height - offset_y);
            }
            for &y in &lines.horizontals {
                MoveToEx(hdc, lines.origin_x - offset_x, y - offset_y, None);
                LineTo(hdc, lines.origin_x + lines.width - offset_x, y - offset_y);
            }
        }

        SelectObject(hdc, old_pen);
        DeleteObject(pen.into());

        // --- 中央線を赤で描画（グリッド座標基準） ---
        let red_pen = CreatePen(PS_SOLID, 2, COLORREF(0x000000FF));
        let old_pen2 = SelectObject(hdc, red_pen.into());

        for monitor in &monitors {
            let grid = monitor.to_grid(&config);
            let lines = grid.grid_lines();

            // 垂直中央線（columns/2 番目のグリッド線）
            let mid_col = grid.columns as usize / 2;
            if mid_col < lines.verticals.len() {
                let x = lines.verticals[mid_col];
                MoveToEx(hdc, x - offset_x, lines.origin_y - offset_y, None);
                LineTo(hdc, x - offset_x, lines.origin_y + lines.height - offset_y);
            }

            // 水平中央線（rows/2 番目のグリッド線）
            let mid_row = grid.rows as usize / 2;
            if mid_row < lines.horizontals.len() {
                let y = lines.horizontals[mid_row];
                MoveToEx(hdc, lines.origin_x - offset_x, y - offset_y, None);
                LineTo(hdc, lines.origin_x + lines.width - offset_x, y - offset_y);
            }
        }

        SelectObject(hdc, old_pen2);
        DeleteObject(red_pen.into());
    }
}