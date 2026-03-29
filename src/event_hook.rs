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
/// ドラッグ中はマウス座標からグリッドを算出しオーバーレイを表示する。
/// マウスリリースでオーバーレイを非表示にする。
unsafe extern "C" fn drag_event_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    _user_info: *mut c_void,
) -> CGEventRef {
    match event_type {
        kCGEventLeftMouseDragged => {
            DRAGGING.store(true, Ordering::SeqCst);
            let pos = CGEventGetLocation(event);
            let state_guard = STATE.lock().unwrap();
            if let Some(s) = state_guard.as_ref() {
                if let Some(monitor) = mac_monitor::monitor_for_point(pos.x, pos.y) {
                    let grid = monitor.to_grid(&s.config);
                    if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                        ov.show(&grid);
                    }
                }
            }
        }
        kCGEventLeftMouseUp => {
            if DRAGGING.load(Ordering::SeqCst) {
                DRAGGING.store(false, Ordering::SeqCst);
                if let Some(ov) = OVERLAY.lock().unwrap().as_ref() {
                    ov.hide();
                }
                // スナップは AXObserver の AXWindowMoved (リリース後に発火) に委譲
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
            // ドラッグ中（CGEventTap が DRAGGING を管理）はスナップを保留し、
            // ウィンドウ参照だけ記録する。オーバーレイ表示は CGEventTap 側が担当。
            if DRAGGING.load(Ordering::SeqCst) {
                // 保留に保存（既存があれば CFRelease）
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
                // マウスリリース済み → 即座にスナップ
                snap_window(element);
            }
        }
        "AXWindowCreated" => {
            eprintln!("[GridSnap] AXWindowCreated detected");
            // F0: 新規ウィンドウ → グリッドにスナップ
            snap_window(element);
        }
        _ => {}
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

    let monitor = match mac_monitor::monitor_for_point(x, y) {
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
        // 最後のイベントから 100ms 経過かつマウスリリース済み
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