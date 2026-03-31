//! macOS イベントフック管理 (AXObserver + CFRunLoop).
//! Windows の event_hook.rs に対応する macOS 実装。
//!
//! 設計:
//! - 各 GUI アプリの PID ごとに AXObserver を作成
//! - AXWindowMoved / AXWindowResized / AXWindowCreated を購読
//! - ドラッグ開始時にオーバーレイを1回表示し、ドラッグ中は処理しない
//! - マルチモニタードラッグ時はモニター境界越え時のみオーバーレイ更新
//! - マウスリリース時にスナップを適用
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

/// スナップ対象のウィンドウかどうかを判定する。
/// AXStandardWindow のみを対象とし、IME候補・ポップアップ・ダイアログを除外。
fn is_snappable_window(element: AXUIElementRef) -> bool {
    unsafe {
        let attr = cf_str("AXSubrole");
        let mut value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr, &mut value);
        CFRelease(attr);
        if err != kAXErrorSuccess || value.is_null() {
            return false;
        }
        let subrole = cf_string_to_string(value as CFStringRef);
        CFRelease(value);
        subrole == "AXStandardWindow"
    }
}

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
static WINDOW_DRAGGING: AtomicBool = AtomicBool::new(false);

/// 現在オーバーレイを表示中のモニター境界（CG座標）。
/// ドラッグ中のモニター境界越え判定に使用する。
static OVERLAY_BOUNDS: Mutex<Option<CGRect>> = Mutex::new(None);

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

        // CGEventTap: ドラッグ開始/終了・モニター境界越えを検出
        install_event_tap();

        // 150ms タイマー: 保留スナップの安全ネット（LeftMouseUp 取りこぼし対策）
        create_timer(0.15, snap_check_callback);

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

// ──── CGEventTap ────

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
/// ドラッグ中: モニター境界越え時のみオーバーレイ更新。
/// マウスアップ: オーバーレイ非表示 + スナップ実行。
unsafe extern "C" fn drag_event_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match event_type {
        kCGEventLeftMouseDragged => {
            DRAGGING.store(true, Ordering::SeqCst);

            // ウィンドウドラッグ中のみ、モニター境界越えをチェック
            if WINDOW_DRAGGING.load(Ordering::SeqCst) {
                let point = CGEventGetLocation(event);
                let needs_update = {
                    let bounds_guard = OVERLAY_BOUNDS.lock().unwrap();
                    match bounds_guard.as_ref() {
                        Some(bounds) => {
                            let inside = point.x >= bounds.origin.x
                                && point.x < bounds.origin.x + bounds.size.width
                                && point.y >= bounds.origin.y
                                && point.y < bounds.origin.y + bounds.size.height;
                            !inside
                        }
                        None => true,
                    }
                }; // bounds_guard ロック解放
                if needs_update {
                    show_overlay_for_point(point.x, point.y);
                }
            }
        }
        kCGEventLeftMouseUp => {
            let was_window_dragging = WINDOW_DRAGGING.swap(false, Ordering::SeqCst);
            let was_dragging = DRAGGING.swap(false, Ordering::SeqCst);

            if was_dragging {
                // オーバーレイ非表示
                if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                    ov.hide();
                }
                *OVERLAY_BOUNDS.lock().unwrap() = None;

                // ウィンドウドラッグだった場合、スナップ適用
                if was_window_dragging {
                    let pending = PENDING.lock().unwrap().take();
                    if let Some(p) = pending {
                        snap_window(p.window.as_ax());
                        CFRelease(p.window.0);
                    }
                }
            }
        }
        _ => {}
    }
    event
}

/// 指定座標のモニターでオーバーレイを表示し、OVERLAY_BOUNDS を更新する。
fn show_overlay_for_point(x: f64, y: f64) {
    let state_guard = STATE.lock().unwrap();
    if let Some(s) = state_guard.as_ref() {
        if let Some(monitor) = mac_monitor::monitor_for_point(x, y)
            .or_else(|| mac_monitor::monitor_nearest_point(x, y))
        {
            let grid = monitor.to_grid(&s.config);
            *OVERLAY_BOUNDS.lock().unwrap() = Some(monitor.bounds);
            if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                ov.show(&grid);
            }
        }
    }
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
            if !is_snappable_window(element) {
                return;
            }
            if DRAGGING.load(Ordering::SeqCst) {
                // ドラッグ中: 初回のみオーバーレイ表示、以後は PENDING 更新のみ
                let was_window_dragging = WINDOW_DRAGGING.swap(true, Ordering::SeqCst);

                if !was_window_dragging {
                    // 初回検出: ウィンドウ位置からモニターを特定しオーバーレイ表示
                    if let Some((wx, wy)) = mac_snap::get_window_position(element) {
                        show_overlay_for_point(wx, wy);
                    }
                }

                // PENDING を更新（スナップ対象ウィンドウの参照を保持）
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
                // 非ドラッグ（キーボード操作等）: 即座にスナップ
                snap_window(element);
            }
        }
        "AXWindowCreated" => {
            if !is_snappable_window(element) {
                return;
            }
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

        // 3. ウィンドウ位置からモニターを特定（ルールマッチングに先行して必要）
        let (wx, wy) = match mac_snap::get_window_position(element) {
            Some(pos) => pos,
            None => return false,
        };
        let monitor = match mac_monitor::monitor_for_point(wx, wy)
            .or_else(|| mac_monitor::monitor_nearest_point(wx, wy))
        {
            Some(m) => m,
            None => return false,
        };
        let monitor_key = format!("{}", monitor.display_id);

        // 4. app_rules で 2 パスマッチング（Windows auto_place.rs と同じ戦略）
        //    Pass 1: モニター固有ルール（monitor フィールド部分一致）
        //    Pass 2: 共通ルール（monitor = None）
        let (rule_col, rule_row, rule_cs, rule_rs, grid) = {
            let state = STATE.lock().unwrap();
            let s = match state.as_ref() {
                Some(s) => s,
                None => return false,
            };

            let matches_app = |rule: &crate::config::AppRule| -> bool {
                if let Some(ref exe) = rule.exe_name {
                    let exe_lower = exe.to_lowercase();
                    let name_lower = app_name.to_lowercase();
                    name_lower.contains(&exe_lower) || exe_lower.contains(&name_lower)
                } else {
                    false
                }
            };

            // Pass 1: モニター固有ルール
            let rule = s.config.app_rules.iter().find(|r| {
                if let Some(ref mon) = r.monitor {
                    monitor_key.contains(mon.as_str()) && matches_app(r)
                } else {
                    false
                }
            });

            // Pass 2: 共通ルール（monitor = None）
            let rule = rule.or_else(|| {
                s.config.app_rules.iter().find(|r| {
                    r.monitor.is_none() && matches_app(r)
                })
            });

            let rule = match rule {
                Some(r) => r,
                None => return false,
            };

            let grid = monitor.to_grid(&s.config);
            (rule.col, rule.row, rule.col_span, rule.row_span, grid)
        }; // STATE lock 解放

        // 5. cell_rect でターゲット位置を計算し、配置
        let rect = grid.cell_rect(rule_col, rule_row, rule_cs, rule_rs);
        mac_snap::set_window_size(element, rect.w as f64, rect.h as f64);
        mac_snap::set_window_position(element, rect.x as f64, rect.y as f64);

        eprintln!(
            "[GridSnap] F0 auto-place: '{}' [mon={}] -> col={}, row={}, span={}x{} pos=({},{})",
            app_name, monitor_key, rule_col, rule_row, rule_cs, rule_rs, rect.x, rect.y
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

/// 150ms タイマー: LeftMouseUp 取りこぼし時の安全ネット。
/// WINDOW_DRAGGING が true のままマウスが離されている場合にフォールバック処理する。
extern "C" fn snap_check_callback(
    _timer: CFRunLoopTimerRef,
    _info: *mut c_void,
) {
    if WINDOW_DRAGGING.load(Ordering::SeqCst)
        && !DRAGGING.load(Ordering::SeqCst)
        && !mac_snap::is_mouse_down()
    {
        WINDOW_DRAGGING.store(false, Ordering::SeqCst);

        if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
            ov.hide();
        }
        *OVERLAY_BOUNDS.lock().unwrap() = None;

        let pending = PENDING.lock().unwrap().take();
        if let Some(p) = pending {
            snap_window(p.window.as_ax());
            unsafe { CFRelease(p.window.0) };
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