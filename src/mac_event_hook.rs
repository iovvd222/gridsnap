//! macOS イベントフック管理 (AXObserver + CFRunLoop).
//! Windows の event_hook.rs に対応する macOS 実装。
//!
//! 設計:
//! - 各 GUI アプリの PID ごとに AXObserver を作成
//! - AXWindowMoved / AXWindowResized / AXWindowCreated を購読
//! - ドラッグ中（マウスボタン押下中）はスナップを保留し、
//!   150ms タイマーでマウスリリース後にスナップを適用（デバウンス）
//! - 3秒タイマーで新規アプリを検出し Observer を追加

use anyhow::Result;
use std::collections::HashMap;
use std::sync::Mutex;
use std::os::raw::c_void;

use crate::config::Config;
use crate::mac_ffi::*;
use crate::mac_monitor;
use crate::mac_snap;
use std::sync::atomic::{AtomicBool, Ordering};

// ──── Send ラッパー（CoreFoundation ポインタ用）────
// macOS の AX/CF ポインタはメインスレッド専用だが、
// 本アプリはシングルスレッド（CFRunLoop）で動作するため安全。

#[derive(Clone, Copy)]
struct SendPtr(*const c_void);
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

impl SendPtr {
    fn as_ax(self) -> AXUIElementRef { self.0 }
    fn as_observer(self) -> AXObserverRef { self.0 }
}

// ──── Global state ────

struct HookState {
    config: Config,
    observers: HashMap<i32, SendPtr>, // pid -> observer (wrapped)
}

/// 保留中のスナップ情報
struct PendingSnap {
    window: SendPtr, // CFRetain 済み
    timestamp: f64,
}

static STATE: Mutex<Option<HookState>> = Mutex::new(None);
static PENDING: Mutex<Option<PendingSnap>> = Mutex::new(None);
static OVERLAY: Mutex<Option<crate::mac_overlay::OverlayWindow>> = Mutex::new(None);

/// ドラッグ中フラグ（CGEventTap が管理）。
/// ax_callback はこのフラグを参照してオーバーレイ制御をスキップする。
static DRAGGING: AtomicBool = AtomicBool::new(false);

/// ウィンドウ操作確認フラグ。
/// ax_callback が DRAGGING 中に AXWindowMoved/AXWindowResized を受信したら true。
/// true になった後は drag_event_callback がマウス座標でオーバーレイを滑らかに駆動する。
static WINDOW_DRAGGING: AtomicBool = AtomicBool::new(false);

/// ドラッグ中のマウス座標（CGEventTap が更新、タイマーが参照）。
static MOUSE_POS: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

// ──── Public API ────

pub struct EventHookManager;

impl EventHookManager {
    pub fn new(config: Config) -> Result<Self> {
        // アクセシビリティ権限チェック
        if !unsafe { AXIsProcessTrusted() } {
            eprintln!("[GridSnap] ⚠️  アクセシビリティ権限が必要です。");
            eprintln!(
                "[GridSnap] システム設定 → プライバシーとセキュリティ → アクセシビリティ で GridSnap を許可してください。"
            );
            anyhow::bail!(
                "Accessibility permission not granted. \
                 Enable in System Settings → Privacy & Security → Accessibility."
            );
        }
        eprintln!("[GridSnap] Accessibility permission OK");

        *STATE.lock().unwrap() = Some(HookState {
            config,
            observers: HashMap::new(),
        });

        // NSPanel を作る前に NSApplication を初期化する。
        // [NSApplication sharedApplication] は冪等なので run_loop() と二重呼び出しになっても安全。
        // これより前に呼ばれていないと initWithContentRect が nil を返し overlay が機能しない。
        unsafe {
            use objc::{class, msg_send, sel, sel_impl};
            let app: cocoa::base::id = msg_send![class!(NSApplication), sharedApplication];
            // AccessoryPolicy: Dock に出さず、フォーカスを奪わない
            let _: () = msg_send![app, setActivationPolicy:1isize];
        }

        match crate::mac_overlay::OverlayWindow::new() {
            Ok(ov) => {
                eprintln!("[GridSnap] mac_overlay created OK");
                *OVERLAY.lock().unwrap() = Some(ov);
            }
            Err(e) => {
                eprintln!("[GridSnap] mac_overlay creation FAILED: {:?}", e);
            }
        }

        // 現在実行中の全 GUI アプリに Observer を登録
        let pids = get_gui_app_pids();
        eprintln!("[GridSnap] Found {} GUI apps", pids.len());
        for pid in &pids {
            register_observer(*pid);
        }

        // CGEventTap: ドラッグ中のオーバーレイ表示を駆動
        install_event_tap();

        // 150ms タイマー: 保留スナップのチェック
        create_timer(0.15, snap_check_callback);

        // 16ms タイマー: ウィンドウドラッグ中のオーバーレイ更新 (~60fps)
        create_timer(0.016, overlay_update_callback);

        // 3秒タイマー: 新規アプリスキャン
        create_timer(3.0, app_scan_callback);

        Ok(Self)
    }
}

/// NSApplication の run_loop を実行する（ブロッキング）。
pub fn run_loop() {
    eprintln!("[GridSnap] Starting NSApplication run loop...");
    unsafe {
        use objc::{class, msg_send, sel, sel_impl};
        let app: cocoa::base::id = msg_send![class!(NSApplication), sharedApplication];
        // NSApplicationActivationPolicyAccessory = 1
        let _: () = msg_send![app, setActivationPolicy:1isize];
        let _: () = msg_send![app, run];
    }
}

// ──── CGEventTap (ドラッグ中オーバーレイ) ────

/// CGEventTap を作成し、メイン RunLoop に登録する。
/// kCGEventLeftMouseDragged / kCGEventLeftMouseUp を ListenOnly で監視。
fn install_event_tap() {
    unsafe {
        let mask: CGEventMask =
            (1 << kCGEventLeftMouseDragged) | (1 << kCGEventLeftMouseUp);

        let tap = CGEventTapCreate(
            kCGSessionEventTap,
            kCGHeadInsertEventTap,
            kCGEventTapOptionListenOnly,
            mask,
            drag_event_callback,
            std::ptr::null_mut(),
        );
        if tap.is_null() {
            eprintln!("[GridSnap] ⚠️  CGEventTapCreate failed (accessibility permission?)");
            return;
        }

        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0);
        if source.is_null() {
            eprintln!("[GridSnap] ⚠️  CFMachPortCreateRunLoopSource failed");
            return;
        }
        CFRunLoopAddSource(
            CFRunLoopGetCurrent(),
            source,
            kCFRunLoopDefaultMode,
        );
        eprintln!("[GridSnap] CGEventTap installed for drag overlay");
    }
}

/// CGEventTap コールバック。
unsafe extern "C" fn drag_event_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match event_type {
        kCGEventLeftMouseDragged => {
            DRAGGING.store(true, Ordering::SeqCst);
            if WINDOW_DRAGGING.load(Ordering::SeqCst) {
                let point = CGEventGetLocation(event);
                *MOUSE_POS.lock().unwrap() = (point.x, point.y);
            }
        }
        kCGEventLeftMouseUp => {
            WINDOW_DRAGGING.store(false, Ordering::SeqCst);
            if DRAGGING.swap(false, Ordering::SeqCst) {
                if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                    ov.hide();
                }
            }
        }
        _ => {}
    }
    event
}

// ──── Timer creation ────

fn create_timer(interval: f64, callback: CFRunLoopTimerCallBack) {
    unsafe {
        let mut ctx = CFRunLoopTimerContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copy_description: None,
        };
        let timer = CFRunLoopTimerCreate(
            kCFAllocatorDefault,
            CFAbsoluteTimeGetCurrent() + interval,
            interval,
            0,
            0,
            callback,
            &mut ctx,
        );
        CFRunLoopAddTimer(
            CFRunLoopGetCurrent(),
            timer,
            kCFRunLoopDefaultMode,
        );
    }
}

// ──── GUI app PID enumeration ────

/// オンスクリーンの GUI アプリ PID を列挙する（レイヤー 0 のウィンドウ所有者）。
fn get_gui_app_pids() -> Vec<i32> {
    let mut pids = Vec::new();
    unsafe {
        let info = CGWindowListCopyWindowInfo(
            kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        );
        if info.is_null() {
            return pids;
        }
        let count = CFArrayGetCount(info);
        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(info, i) as CFDictionaryRef;
            if dict.is_null() {
                continue;
            }

            // Layer 0（通常ウィンドウ）のみ
            let layer_val =
                CFDictionaryGetValue(dict, kCGWindowLayer as *const c_void);
            if !layer_val.is_null() {
                let mut layer: i32 = 0;
                if CFNumberGetValue(
                    layer_val as CFTypeRef,
                    kCFNumberSInt32Type,
                    &mut layer as *mut _ as *mut c_void,
                ) != 0
                    && layer != 0
                {
                    continue;
                }
            }

            // Owner PID
            let pid_val =
                CFDictionaryGetValue(dict, kCGWindowOwnerPID as *const c_void);
            if !pid_val.is_null() {
                let mut pid: i32 = 0;
                if CFNumberGetValue(
                    pid_val as CFTypeRef,
                    kCFNumberSInt32Type,
                    &mut pid as *mut _ as *mut c_void,
                ) != 0
                    && pid > 0
                    && !pids.contains(&pid)
                {
                    pids.push(pid);
                }
            }
        }
        CFRelease(info as CFTypeRef);
    }
    pids
}

// ──── Observer registration ────

/// 指定 PID に対して AXObserver を登録する。
fn register_observer(pid: i32) {
    let mut state = STATE.lock().unwrap();
    let state = match state.as_mut() {
        Some(s) => s,
        None => return,
    };

    if state.observers.contains_key(&pid) {
        return;
    }

    unsafe {
        let mut observer: AXObserverRef = std::ptr::null();
        let err = AXObserverCreate(pid, ax_callback, &mut observer);
        if err != kAXErrorSuccess || observer.is_null() {
            return;
        }

        let app = AXUIElementCreateApplication(pid);
        if app.is_null() {
            CFRelease(observer);
            return;
        }

        // 通知を登録
        let notifications = [
            "AXWindowMoved",
            "AXWindowResized",
            "AXWindowCreated",
        ];
        for notif_name in &notifications {
            let notif = cf_str(notif_name);
            let err = AXObserverAddNotification(
                observer,
                app,
                notif,
                std::ptr::null_mut(),
            );
            CFRelease(notif);
            if err != kAXErrorSuccess
                && err != kAXErrorNotificationAlreadyRegistered
            {
                eprintln!(
                    "[GridSnap] Failed to add {} for pid {}: err={}",
                    notif_name, pid, err
                );
            }
        }

        // Run loop に追加
        let source = AXObserverGetRunLoopSource(observer);
        if !source.is_null() {
            CFRunLoopAddSource(
                CFRunLoopGetCurrent(),
                source,
                kCFRunLoopDefaultMode,
            );
        }

        CFRelease(app);
        state.observers.insert(pid, SendPtr(observer));
        eprintln!("[GridSnap] Registered observer for pid {}", pid);
    }
}

// ──── AXObserver callback ────

/// AXObserver コールバック。
/// ウィンドウの移動・リサイズ・生成を検出する。
unsafe extern "C" fn ax_callback(
    _observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    _refcon: *mut c_void,
) {
    let notif_str = cf_string_to_string(notification);

    match notif_str.as_str() {
        "AXWindowMoved" | "AXWindowResized" => {
            if DRAGGING.load(Ordering::SeqCst) {
                WINDOW_DRAGGING.store(true, Ordering::SeqCst);
                if let Some((wx, wy)) = mac_snap::get_window_position(element) {
                    let state_guard = STATE.lock().unwrap();
                    if let Some(s) = state_guard.as_ref() {
                        if let Some(monitor) = mac_monitor::monitor_for_point(wx, wy)
                            .or_else(|| mac_monitor::monitor_nearest_point(wx, wy))
                        {
                            let grid = monitor.to_grid(&s.config);
                            if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                                ov.show(&grid);
                            }
                        }
                    }
                }

                let mut pending = PENDING.lock().unwrap();
                if let Some(old) = pending.take() {
                    CFRelease(old.window.0);
                }
                CFRetain(element);
                *pending = Some(PendingSnap {
                    window: SendPtr(element),
                    timestamp: CFAbsoluteTimeGetCurrent(),
                });
            } else {
                snap_window(element);
            }
        }
        "AXWindowCreated" => {
            eprintln!("[GridSnap] AXWindowCreated detected");
            // F0: app_rules にマッチすれば指定セルに配置、そうでなければグリッドスナップ
            if !try_auto_place(element) {
                snap_window(element);
            }
        }
        _ => {}
    }
}

/// F0: app_rules に基づく自動配置を試みる。マッチしたら true、しなければ false。
fn try_auto_place(element: AXUIElementRef) -> bool {
    unsafe {
        // 1. AXUIElement から PID を取得
        let mut pid: i32 = 0;
        if AXUIElementGetPid(element, &mut pid) != kAXErrorSuccess {
            return false;
        }

        // 2. NSRunningApplication からアプリ名を取得
        use cocoa::base::{id, nil};
        use objc::{class, msg_send, sel, sel_impl};

        let running_app: id = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier:pid
        ];
        if running_app == nil {
            return false;
        }
        let ns_name: id = msg_send![running_app, localizedName];
        let app_name = if ns_name != nil {
            let c_str: *const i8 = msg_send![ns_name, UTF8String];
            if !c_str.is_null() {
                std::ffi::CStr::from_ptr(c_str)
                    .to_string_lossy()
                    .into_owned()
            } else {
                return false;
            }
        } else {
            return false;
        };

        // 3. app_rules でマッチング（部分一致、Windows 版と同じロジック）
        let (rule_col, rule_row, rule_cs, rule_rs, grid) = {
            let state = STATE.lock().unwrap();
            let s = match state.as_ref() {
                Some(s) => s,
                None => return false,
            };

            let rule = s.config.app_rules.iter().find(|r| {
                if let Some(ref exe) = r.exe_name {
                    let exe_lower = exe.to_lowercase();
                    let name_lower = app_name.to_lowercase();
                    name_lower.contains(&exe_lower) || exe_lower.contains(&name_lower)
                } else {
                    false
                }
            });

            let rule = match rule {
                Some(r) => r,
                None => return false,
            };

            // ウィンドウ位置からモニターを特定
            let (x, y) = match mac_snap::get_window_position(element) {
                Some(pos) => pos,
                None => return false,
            };
            let monitor = match mac_monitor::monitor_for_point(x, y)
                .or_else(|| mac_monitor::monitor_nearest_point(x, y))
            {
                Some(m) => m,
                None => return false,
            };
            let grid = monitor.to_grid(&s.config);
            (rule.col, rule.row, rule.col_span, rule.row_span, grid)
        }; // STATE lock 解放

        // 4. cell_rect でターゲット位置を計算し、配置
        let rect = grid.cell_rect(rule_col, rule_row, rule_cs, rule_rs);
        mac_snap::set_window_size(element, rect.w as f64, rect.h as f64);
        mac_snap::set_window_position(element, rect.x as f64, rect.y as f64);

        eprintln!(
            "[GridSnap] F0 auto-place: '{}' -> col={}, row={}, span={}x{} pos=({},{})",
            app_name, rule_col, rule_row, rule_cs, rule_rs, rect.x, rect.y
        );
        true
    }
}

/// ウィンドウにスナップを適用する（グリッド取得を含む）。
fn snap_window(element: AXUIElementRef) {
    let (x, y) = match mac_snap::get_window_position(element) {
        Some(pos) => pos,
        None => return,
    };

    let state = STATE.lock().unwrap();
    let state = match state.as_ref() {
        Some(s) => s,
        None => return,
    };

    let monitor = match mac_monitor::monitor_for_point(x, y)
        .or_else(|| mac_monitor::monitor_nearest_point(x, y))
    {
        Some(m) => m,
        None => return,
    };
    let grid = monitor.to_grid(&state.config);

    mac_snap::apply_snap(element, &grid);
}

// ──── Timer callbacks ────

/// 150ms タイマー: 保留スナップを処理する。
extern "C" fn snap_check_callback(
    _timer: CFRunLoopTimerRef,
    _info: *mut c_void,
) {
    let mut pending = PENDING.lock().unwrap();
    if let Some(p) = pending.as_ref() {
        let now = unsafe { CFAbsoluteTimeGetCurrent() };
        if now - p.timestamp > 0.1 && !mac_snap::is_mouse_down() {
            if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                ov.hide();
            }
            let window = p.window;
            snap_window(window.as_ax());
            unsafe { CFRelease(window.0) };
            *pending = None;
        }
    }
}

/// 16ms タイマー: WINDOW_DRAGGING 中にマウス座標でオーバーレイを更新する。
extern "C" fn overlay_update_callback(
    _timer: CFRunLoopTimerRef,
    _info: *mut c_void,
) {
    if !WINDOW_DRAGGING.load(Ordering::SeqCst) || !DRAGGING.load(Ordering::SeqCst) {
        return;
    }
    let (mx, my) = *MOUSE_POS.lock().unwrap();
    if mx == 0.0 && my == 0.0 {
        return;
    }
    let grid = {
        let state_guard = STATE.lock().unwrap();
        match state_guard.as_ref() {
            Some(s) => mac_monitor::monitor_for_point(mx, my)
                .or_else(|| mac_monitor::monitor_nearest_point(mx, my))
                .map(|m| m.to_grid(&s.config)),
            None => None,
        }
    };
    if let Some(grid) = grid {
        if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
            ov.show(&grid);
        }
    }
}

/// 3秒タイマー: 新規アプリをスキャンする。
extern "C" fn app_scan_callback(
    _timer: CFRunLoopTimerRef,
    _info: *mut c_void,
) {
    let pids = get_gui_app_pids();
    for pid in pids {
        register_observer(pid);
    }
}

// ──── Public: Config の動的更新 ────

/// 外部（mac_tray）から Config を更新する。
/// クロージャ内で Config を変更し、変更後の Config を TOML に保存する。
pub fn update_config<F: FnOnce(&mut Config)>(f: F) {
    let mut state = STATE.lock().unwrap();
    if let Some(s) = state.as_mut() {
        f(&mut s.config);
        if let Err(e) = s.config.save() {
            eprintln!("[GridSnap] Failed to save config: {:?}", e);
        }
        eprintln!(
            "[GridSnap] Config updated: {}x{}",
            s.config.grid.columns, s.config.grid.rows
        );
    }
}

/// F0a: フロントウィンドウのアプリ名と位置をキャプチャし、app_rules に upsert する。
/// トレイメニューの「Capture Position」から呼ばれる。
pub fn capture_frontmost_window() {
    unsafe {
        use cocoa::base::{id, nil};
        use objc::{class, msg_send, sel, sel_impl};

        // 1. フロントアプリの PID と名前を取得
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let front_app: id = msg_send![workspace, frontmostApplication];
        if front_app == nil {
            eprintln!("[GridSnap] capture: no frontmost app");
            return;
        }
        let pid: i32 = msg_send![front_app, processIdentifier];
        let ns_name: id = msg_send![front_app, localizedName];
        let app_name = if ns_name != nil {
            let c_str: *const i8 = msg_send![ns_name, UTF8String];
            if !c_str.is_null() {
                std::ffi::CStr::from_ptr(c_str)
                    .to_string_lossy()
                    .into_owned()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        if app_name.is_empty() {
            eprintln!("[GridSnap] capture: could not get app name for pid {}", pid);
            return;
        }

        // 2. AX API でフォーカスウィンドウの位置・サイズを取得
        let ax_app = AXUIElementCreateApplication(pid);
        if ax_app.is_null() {
            eprintln!("[GridSnap] capture: AXUIElementCreateApplication failed");
            return;
        }
        let attr = cf_str("AXFocusedWindow");
        let mut window: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(ax_app, attr, &mut window);
        CFRelease(attr);
        CFRelease(ax_app);
        if err != kAXErrorSuccess || window.is_null() {
            eprintln!("[GridSnap] capture: no focused window for '{}'", app_name);
            return;
        }

        let pos = mac_snap::get_window_position(window);
        let size = mac_snap::get_window_size(window);
        CFRelease(window);

        let (wx, wy) = match pos {
            Some(p) => p,
            None => {
                eprintln!("[GridSnap] capture: failed to get window position");
                return;
            }
        };
        let (ww, wh) = match size {
            Some(s) => s,
            None => {
                eprintln!("[GridSnap] capture: failed to get window size");
                return;
            }
        };

        // 3. モニター検出 -> グリッド -> セル座標に変換
        let monitor = match mac_monitor::monitor_for_point(wx, wy)
            .or_else(|| mac_monitor::monitor_nearest_point(wx, wy))
        {
            Some(m) => m,
            None => {
                eprintln!("[GridSnap] capture: no monitor at ({}, {})", wx, wy);
                return;
            }
        };

        let grid = {
            let state = STATE.lock().unwrap();
            match state.as_ref() {
                Some(s) => monitor.to_grid(&s.config),
                None => {
                    eprintln!("[GridSnap] capture: STATE not initialized");
                    return;
                }
            }
        };

        let (col, row, col_span, row_span) = grid.rect_to_cell(wx, wy, ww, wh);
        let monitor_key = format!("{}", monitor.display_id);

        // 4. app_rules に upsert -> TOML 保存
        //    キャプチャ時のモニターを記録し、モニター固有ルールとして保存する。
        let rule = crate::config::AppRule {
            monitor: Some(monitor_key.clone()),
            class_name: None,
            exe_name: Some(app_name.clone()),
            col,
            row,
            col_span,
            row_span,
        };

        update_config(|config| {
            config.upsert_app_rule(rule);
        });

        eprintln!(
            "[GridSnap] Captured: '{}' -> col={}, row={}, span={}x{}",
            app_name, col, row, col_span, row_span
        );

        // トースト表示（モデル名付き）
        let monitor_name = mac_monitor::display_name(monitor.display_id);
        let msg = format!(
            "Captured: {} [{}] -> ({},{}) {}x{}",
            app_name, monitor_name, col, row, col_span, row_span
        );
        if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
            ov.show_toast(&msg);
        }
    }
}