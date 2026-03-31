//! Windows システムトレイ（Shell_NotifyIconW + ポップアップメニュー）。
//! macOS の mac_tray.rs に対応する Windows 実装。
//!
//! 機能:
//! - Columns / Rows のプリセット変更（サブメニュー）
//! - Capture Position（F0a: フォアグラウンドウィンドウの位置を app_rules に記録）
//! - Quit

use std::sync::Once;
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIF_INFO,
        NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    },
    UI::WindowsAndMessaging::{
        CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow,
        FindWindowW, GetCursorPos, LoadImageW,
        PostQuitMessage, RegisterClassExW, SetForegroundWindow, TrackPopupMenu,
        AppendMenuW, CheckMenuRadioItem,
        CS_HREDRAW, CS_VREDRAW, HMENU, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED,
        MF_BYCOMMAND, MF_POPUP, MF_SEPARATOR, MF_STRING,
        TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON,
        WM_APP, WM_COMMAND, WM_DESTROY, WNDCLASSEXW, WS_EX_TOOLWINDOW,
        WS_OVERLAPPEDWINDOW,
    },
};
use windows_core::PCWSTR;

use crate::config::Config;

// ──── 定数 ────

const WM_TRAYICON: u32 = WM_APP + 1;

/// メニューコマンド ID
const ID_COL_BASE: u16 = 1000;
const ID_ROW_BASE: u16 = 2000;
const ID_CAPTURE: u16 = 8000;
const ID_QUIT: u16 = 9999;

const COLUMN_PRESETS: &[u32] = &[4, 6, 8, 12, 16, 20];
const ROW_PRESETS: &[u32] = &[2, 3, 4, 6, 8, 12];

const TRAY_CLASS: &str = "GridSnapTrayWindow";

static REGISTER_CLASS: Once = Once::new();

// ──── 現在のグリッド設定（チェックマーク用）────
static CURRENT_COLS: std::sync::Mutex<u32> = std::sync::Mutex::new(20);
static CURRENT_ROWS: std::sync::Mutex<u32> = std::sync::Mutex::new(12);


/// トレイを初期化し、隠しウィンドウを作成する。
/// メッセージループは main.rs の既存 message_loop() で処理される。
pub fn setup(config: &Config) -> Option<HWND> {
    *CURRENT_COLS.lock().unwrap() = config.grid.columns;
    *CURRENT_ROWS.lock().unwrap() = config.grid.rows;

    register_class();

    unsafe {
        let hinstance = GetModuleHandleW(None).ok()?;
        let class_name = to_wide(TRAY_CLASS);

        // 隠しウィンドウ（メッセージ受信用）
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WS_OVERLAPPEDWINDOW,
            0, 0, 0, 0,
            None, None, Some(hinstance.into()), None,
        ).ok()?;

        // Shell_NotifyIcon
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAYICON;

        // 標準アプリアイコンを使う（IDI_APPLICATION = 32512）
        let icon = LoadImageW(
            None,
            PCWSTR(32512 as *const u16),
            IMAGE_ICON,
            0, 0,
            LR_DEFAULTSIZE | LR_SHARED,
        );
        if let Ok(icon) = icon {
            nid.hIcon = std::mem::transmute(icon);
        }

        let tip = "GridSnap";
        let tip_wide: Vec<u16> = tip.encode_utf16().collect();
        let copy_len = tip_wide.len().min(nid.szTip.len() - 1);
        nid.szTip[..copy_len].copy_from_slice(&tip_wide[..copy_len]);

        Shell_NotifyIconW(NIM_ADD, &nid);

        eprintln!(
            "[GridSnap] System tray installed ({}x{})",
            config.grid.columns, config.grid.rows
        );

        Some(hwnd)
    }
}

/// トレイアイコンを削除する。
pub fn cleanup(hwnd: HWND) {
    unsafe {
        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &nid);
        let _ = DestroyWindow(hwnd);
    }
}

// ──── ウィンドウクラス登録 ────

fn register_class() {
    REGISTER_CLASS.call_once(|| unsafe {
        let hinstance = GetModuleHandleW(None).unwrap();
        let class_name = to_wide(TRAY_CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(tray_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        // class_name は 'static ではないが、REGISTER_CLASS の Once で
        // 二度呼ばれないため問題ない。
        // ただし PCWSTR のライフタイムに注意: ここでは RegisterClassExW が
        // 内部にコピーするので解放しても安全。
    });
}

// ──── ウィンドウプロシージャ ────

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_TRAYICON => {
            // lparam の下位ワード: マウスメッセージ
            let mouse_msg = (lparam.0 as u32) & 0xFFFF;
            // WM_RBUTTONUP = 0x0205, WM_LBUTTONUP = 0x0202
            if mouse_msg == 0x0205 || mouse_msg == 0x0202 {
                show_context_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let cmd_id = (wparam.0 as u32) & 0xFFFF;
            handle_command(cmd_id as u16);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

// ──── コンテキストメニュー ────

fn show_context_menu(hwnd: HWND) {
    unsafe {
        let menu = CreatePopupMenu().unwrap();
        let current_cols = *CURRENT_COLS.lock().unwrap();
        let current_rows = *CURRENT_ROWS.lock().unwrap();

        // ── Columns サブメニュー ──
        let col_menu = CreatePopupMenu().unwrap();
        for &val in COLUMN_PRESETS {
            let label = to_wide(&format!("{}", val));
            AppendMenuW(
                col_menu,
                MF_STRING,
                (ID_COL_BASE + val as u16) as usize,
                PCWSTR(label.as_ptr()),
            );
        }
        // 現在値にラジオチェック
        let col_first = ID_COL_BASE + COLUMN_PRESETS[0] as u16;
        let col_last = ID_COL_BASE + COLUMN_PRESETS[COLUMN_PRESETS.len() - 1] as u16;
        let col_cur = ID_COL_BASE + current_cols as u16;
        let _ = CheckMenuRadioItem(
            col_menu,
            col_first as u32,
            col_last as u32,
            col_cur as u32,
            MF_BYCOMMAND.0,
        );
        let col_label = to_wide("Columns");
        AppendMenuW(
            menu,
            MF_POPUP,
            col_menu.0 as usize,
            PCWSTR(col_label.as_ptr()),
        );

        // ── Rows サブメニュー ──
        let row_menu = CreatePopupMenu().unwrap();
        for &val in ROW_PRESETS {
            let label = to_wide(&format!("{}", val));
            AppendMenuW(
                row_menu,
                MF_STRING,
                (ID_ROW_BASE + val as u16) as usize,
                PCWSTR(label.as_ptr()),
            );
        }
        let row_first = ID_ROW_BASE + ROW_PRESETS[0] as u16;
        let row_last = ID_ROW_BASE + ROW_PRESETS[ROW_PRESETS.len() - 1] as u16;
        let row_cur = ID_ROW_BASE + current_rows as u16;
        let _ = CheckMenuRadioItem(
            row_menu,
            row_first as u32,
            row_last as u32,
            row_cur as u32,
            MF_BYCOMMAND.0,
        );
        let row_label = to_wide("Rows");
        AppendMenuW(
            menu,
            MF_POPUP,
            row_menu.0 as usize,
            PCWSTR(row_label.as_ptr()),
        );

        // ── Separator ──
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        // ── Capture Position ──
        let capture_label = to_wide("\u{1F4CD} Capture this app's position");
        AppendMenuW(
            menu,
            MF_STRING,
            ID_CAPTURE as usize,
            PCWSTR(capture_label.as_ptr()),
        );

        // ── Separator ──
        AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        // ── Quit ──
        let quit_label = to_wide("Quit GridSnap");
        AppendMenuW(
            menu,
            MF_STRING,
            ID_QUIT as usize,
            PCWSTR(quit_label.as_ptr()),
        );

        // メニュー表示
        let mut pt = windows::Win32::Foundation::POINT::default();
        let _ = GetCursorPos(&mut pt);

        // SetForegroundWindow を呼ばないとメニューが閉じない問題の回避
        let _ = SetForegroundWindow(hwnd);
        TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
            pt.x,
            pt.y,
            Some(0),
            hwnd,
            None,
        );
    }
}

// ──── コマンドハンドラ ────

fn handle_command(cmd_id: u16) {
    // ── Quit ──
    if cmd_id == ID_QUIT {
        eprintln!("[GridSnap] Quit requested from tray");
        unsafe { PostQuitMessage(0) };
        return;
    }

    // ── Capture Position ──
    if cmd_id == ID_CAPTURE {
        eprintln!("[GridSnap] Capture requested from tray");
        if let Some(msg) = crate::event_hook::capture_window(None) {
            // バルーン通知は省略（eprintln で十分）。
            // 必要なら Shell_NotifyIconW + NIF_INFO でバルーンを出せる。
            show_balloon(&msg);
        }
        return;
    }

    // ── Columns preset ──
    if cmd_id >= ID_COL_BASE && cmd_id < ID_ROW_BASE {
        let value = (cmd_id - ID_COL_BASE) as u32;
        eprintln!("[GridSnap] Tray: columns = {}", value);
        *CURRENT_COLS.lock().unwrap() = value;
        crate::event_hook::update_config(|config| {
            config.grid.columns = value;
        });
        return;
    }

    // ── Rows preset ──
    if cmd_id >= ID_ROW_BASE && cmd_id < ID_CAPTURE {
        let value = (cmd_id - ID_ROW_BASE) as u32;
        eprintln!("[GridSnap] Tray: rows = {}", value);
        *CURRENT_ROWS.lock().unwrap() = value;
        crate::event_hook::update_config(|config| {
            config.grid.rows = value;
        });
        return;
    }
}

/// バルーン通知を表示する（キャプチャ結果のフィードバック用）。
fn show_balloon(text: &str) {
    // 注意: バルーンを出すにはトレイの HWND が必要。
    // ここでは簡易実装として NIF_INFO を使う。
    // HWND を static に保持していないため、FindWindowW で取得する。
    unsafe {
        let class_name = to_wide(TRAY_CLASS);
        let hwnd = FindWindowW(PCWSTR(class_name.as_ptr()), PCWSTR::null())
            .unwrap_or_default();
        if hwnd.is_invalid() {
            return;
        }

        let mut nid = NOTIFYICONDATAW::default();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_INFO;

        // バルーンタイトル
        let title = "GridSnap";
        let title_wide: Vec<u16> = title.encode_utf16().collect();
        let copy_len = title_wide.len().min(nid.szInfoTitle.len() - 1);
        nid.szInfoTitle[..copy_len].copy_from_slice(&title_wide[..copy_len]);

        // バルーンテキスト
        let text_wide: Vec<u16> = text.encode_utf16().collect();
        let copy_len = text_wide.len().min(nid.szInfo.len() - 1);
        nid.szInfo[..copy_len].copy_from_slice(&text_wide[..copy_len]);

        Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

// ──── ユーティリティ ────

/// &str を null 終端の UTF-16 Vec に変換する。
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}