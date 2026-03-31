use anyhow::Result;
use cocoa::appkit::{NSColor, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
use cocoa::base::{id, nil, NO, YES};
use cocoa::foundation::{NSPoint, NSRect, NSSize};
use objc::{class, msg_send, sel, sel_impl};

use crate::grid::Grid;

pub struct OverlayWindow {
    panel: id,
    toast_panel: id,
    /// 前回描画したグリッドのキー（origin_x, origin_y, w, h, cols, rows）。
    /// 同一グリッドなら CALayer を再生成しない。
    last_grid_key: std::cell::Cell<(i32, i32, i32, i32, i32, i32)>,
}

unsafe impl Send for OverlayWindow {}
unsafe impl Sync for OverlayWindow {}

impl OverlayWindow {
    pub fn new() -> Result<Self> {
        unsafe {
            let panel: id = msg_send![class!(NSPanel), alloc];
            let rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(100., 100.));
            let style_mask = NSWindowStyleMask::NSBorderlessWindowMask;
            
            let panel: id = msg_send![
                panel,
                initWithContentRect:rect
                styleMask:style_mask
                backing:2 // NSBackingStoreBuffered
                defer:NO
            ];

            // ★ 最重要: NSPanel は hidesOnDeactivate がデフォルト YES。
            // Accessory アプリは永遠に「非アクティブ」なのでパネルが即座に隠れる。
            let _: () = msg_send![panel, setHidesOnDeactivate:NO];

            // 透明・影なし・操作透過
            let color_class = class!(NSColor);
            let clear_color: id = msg_send![color_class, clearColor];
            let _: () = msg_send![panel, setBackgroundColor:clear_color];
            let _: () = msg_send![panel, setOpaque:NO];
            let _: () = msg_send![panel, setHasShadow:NO];
            let _: () = msg_send![panel, setIgnoresMouseEvents:YES];
            let _: () = msg_send![panel, setAlphaValue:1.0f64];
            
            // 最前面レベル
            // kCGScreenSaverWindowLevel(1000) 未満、通常ウィンドウ(0)より十分上
            let _: () = msg_send![panel, setLevel:25i64]; // NSStatusWindowLevel

            // 全スペース参加・Expose無視
            let cb = NSWindowCollectionBehavior::NSWindowCollectionBehaviorCanJoinAllSpaces
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorStationary
                | NSWindowCollectionBehavior::NSWindowCollectionBehaviorIgnoresCycle;
            let _: () = msg_send![panel, setCollectionBehavior:cb];

            // contentView を layer-backed にする（show() より前に確立する）
            let view: id = msg_send![panel, contentView];
            let _: () = msg_send![view, setWantsLayer:YES];

            // ──── Toast panel ────
            let toast: id = msg_send![class!(NSPanel), alloc];
            let toast_rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(340., 40.));
            let toast: id = msg_send![
                toast,
                initWithContentRect:toast_rect
                styleMask:style_mask
                backing:2
                defer:NO
            ];
            let _: () = msg_send![toast, setHidesOnDeactivate:NO];
            let _: () = msg_send![toast, setBackgroundColor:clear_color];
            let _: () = msg_send![toast, setOpaque:NO];
            let _: () = msg_send![toast, setHasShadow:NO];
            let _: () = msg_send![toast, setIgnoresMouseEvents:YES];
            let _: () = msg_send![toast, setAlphaValue:1.0f64];
            let _: () = msg_send![toast, setLevel:25i64];
            let _: () = msg_send![toast, setCollectionBehavior:cb];
            let toast_view: id = msg_send![toast, contentView];
            let _: () = msg_send![toast_view, setWantsLayer:YES];

            Ok(Self {
                panel,
                toast_panel: toast,
                last_grid_key: std::cell::Cell::new((0, 0, 0, 0, 0, 0)),
            })
        }
    }

    /// グリッドオーバーレイを表示する。
    /// **メインスレッドから呼ぶこと。**
    /// ドラッグ中の表示は CGEventTap (mac_event_hook::drag_event_callback) が駆動する。
    pub fn show(&self, grid: &Grid) {
        let key = (
            grid.origin_x, grid.origin_y,
            grid.width, grid.height,
            grid.columns, grid.rows,
        );

        unsafe {
            let panel = self.panel;

            // メインスクリーンの高さを取得（CG座標 top-down → NS座標 bottom-up 変換用）
            // ★ mainScreen はキーフォーカス依存で不安定。
            //   screens[0] は常にプライマリ（メニューバー）ディスプレイを返す。
            //   mac_monitor.rs の CGMainDisplayID() と一致させる。
            let screens: id = msg_send![class!(NSScreen), screens];
            let primary_screen: id = msg_send![screens, objectAtIndex:0usize];
            let main_frame: NSRect = msg_send![primary_screen, frame];
            let main_h = main_frame.size.height;

            // CG座標(top-down, origin=左上) → NS座標(bottom-up, origin=左下) 変換
            let frame_x = grid.origin_x as f64;
            let frame_y = main_h - (grid.origin_y as f64 + grid.height as f64);

            let rect = NSRect::new(
                NSPoint::new(frame_x, frame_y),
                NSSize::new(grid.width as f64, grid.height as f64),
            );
            let _: () = msg_send![panel, setFrame:rect display:YES];

            // グリッドが変わっていなければ CALayer を再構築しない
            if self.last_grid_key.get() != key {
                self.last_grid_key.set(key);
                self.rebuild_layers(grid);
            }

            let _: () = msg_send![panel, orderFrontRegardless];
            let _: () = msg_send![panel, display];

            eprintln!("[GridSnap] overlay shown: x={} y={} w={} h={}",
                frame_x, frame_y, grid.width, grid.height);
        }
    }

    /// CALayer でグリッド線を再構築する。
    unsafe fn rebuild_layers(&self, grid: &Grid) {
        let view: id = msg_send![self.panel, contentView];
        let root_layer: id = msg_send![view, layer];

        let _: () = msg_send![root_layer, setSublayers:nil];
        let _: () = msg_send![root_layer, setGeometryFlipped:YES];

        // CGColor を CoreGraphics API で直接作成（autorelease 依存を排除）
        let color_space: id = CGColorSpaceCreateDeviceRGB();
        let components: [f64; 4] = [1.0, 0.5, 0.0, 0.8]; // orange, alpha=0.8
        let cg_orange: id = CGColorCreate(color_space, components.as_ptr());
        CGColorSpaceRelease(color_space);

        let layer_class = class!(CALayer);
        let lines = grid.grid_lines();
        let thickness = 2.0_f64;

        for &x in &lines.verticals {
            let local_x = (x - grid.origin_x) as f64;
            let line: id = msg_send![layer_class, layer];
            let _: () = msg_send![line, setBackgroundColor:cg_orange];
            let lf = NSRect::new(
                NSPoint::new(local_x, 0.),
                NSSize::new(thickness, grid.height as f64),
            );
            let _: () = msg_send![line, setFrame:lf];
            let _: () = msg_send![root_layer, addSublayer:line];
        }

        for &y in &lines.horizontals {
            let local_y = (y - grid.origin_y) as f64;
            let line: id = msg_send![layer_class, layer];
            let _: () = msg_send![line, setBackgroundColor:cg_orange];
            let lf = NSRect::new(
                NSPoint::new(0., local_y),
                NSSize::new(grid.width as f64, thickness),
            );
            let _: () = msg_send![line, setFrame:lf];
            let _: () = msg_send![root_layer, addSublayer:line];
        }

        CGColorRelease(cg_orange);

        // --- 中央線を赤で描画 ---
        let color_space2: id = CGColorSpaceCreateDeviceRGB();
        let red_components: [f64; 4] = [1.0, 0.0, 0.0, 0.9]; // red, alpha=0.9
        let cg_red: id = CGColorCreate(color_space2, red_components.as_ptr());
        CGColorSpaceRelease(color_space2);

        let center_thickness = 3.0_f64;

        // 垂直中央線
        let mid_col = grid.columns as usize / 2;
        if mid_col < lines.verticals.len() {
            let local_x = (lines.verticals[mid_col] - grid.origin_x) as f64;
            let line: id = msg_send![layer_class, layer];
            let _: () = msg_send![line, setBackgroundColor:cg_red];
            let lf = NSRect::new(
                NSPoint::new(local_x, 0.),
                NSSize::new(center_thickness, grid.height as f64),
            );
            let _: () = msg_send![line, setFrame:lf];
            let _: () = msg_send![root_layer, addSublayer:line];
        }

        // 水平中央線
        let mid_row = grid.rows as usize / 2;
        if mid_row < lines.horizontals.len() {
            let local_y = (lines.horizontals[mid_row] - grid.origin_y) as f64;
            let line: id = msg_send![layer_class, layer];
            let _: () = msg_send![line, setBackgroundColor:cg_red];
            let lf = NSRect::new(
                NSPoint::new(0., local_y),
                NSSize::new(grid.width as f64, center_thickness),
            );
            let _: () = msg_send![line, setFrame:lf];
            let _: () = msg_send![root_layer, addSublayer:line];
        }

        CGColorRelease(cg_red);

        let _: () = msg_send![root_layer, setNeedsLayout];
        let _: () = msg_send![root_layer, layoutIfNeeded];
    }

    /// トーストメッセージを画面中央に表示し、1.5秒後に自動消去する。
    /// メインスレッドから呼ぶこと。
    pub fn show_toast(&self, message: &str) {
        unsafe {
            let panel = self.toast_panel;

            // 前回の自動消去タイマーをキャンセル
            let _: () = msg_send![
                class!(NSObject),
                cancelPreviousPerformRequestsWithTarget:panel
                selector:sel!(orderOut:)
                object:nil
            ];

            // メインスクリーン中央に配置
            let screens: id = msg_send![class!(NSScreen), screens];
            let main_screen: id = msg_send![screens, objectAtIndex:0usize];
            let main_frame: NSRect = msg_send![main_screen, frame];
            let tw = 400.0_f64;
            let th = 52.0_f64;
            let tx = main_frame.origin.x + (main_frame.size.width - tw) / 2.0;
            let ty = main_frame.origin.y + (main_frame.size.height - th) / 2.0;
            let rect = NSRect::new(NSPoint::new(tx, ty), NSSize::new(tw, th));
            let _: () = msg_send![panel, setFrame:rect display:YES];

            // レイヤー再構築
            let view: id = msg_send![panel, contentView];
            let root: id = msg_send![view, layer];
            let _: () = msg_send![root, setSublayers:nil];

            // 背景レイヤー（ダーク半透明・角丸・白枠）
            let bg: id = msg_send![class!(CALayer), layer];
            let bg_rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(tw, th));
            let _: () = msg_send![bg, setFrame:bg_rect];
            let _: () = msg_send![bg, setCornerRadius:10.0f64];
            let cs = CGColorSpaceCreateDeviceRGB();
            let bg_c: [f64; 4] = [0.10, 0.10, 0.10, 0.92];
            let bg_color = CGColorCreate(cs, bg_c.as_ptr());
            let _: () = msg_send![bg, setBackgroundColor:bg_color];
            CGColorRelease(bg_color);

            // 白ボーダー
            let border_c: [f64; 4] = [1.0, 1.0, 1.0, 0.6];
            let border_color = CGColorCreate(cs, border_c.as_ptr());
            let _: () = msg_send![bg, setBorderColor:border_color];
            CGColorRelease(border_color);
            let _: () = msg_send![bg, setBorderWidth:1.5f64];

            // テキストレイヤー
            let text: id = msg_send![class!(CATextLayer), layer];
            let text_rect = NSRect::new(NSPoint::new(16., 10.), NSSize::new(tw - 32., th - 20.));
            let _: () = msg_send![text, setFrame:text_rect];

            let cf_msg = crate::mac_ffi::cf_str(message);
            let _: () = msg_send![text, setString:cf_msg];
            crate::mac_ffi::CFRelease(cf_msg as crate::mac_ffi::CFTypeRef);

            let _: () = msg_send![text, setFontSize:15.0f64];
            let align = crate::mac_ffi::cf_str("center");
            let _: () = msg_send![text, setAlignmentMode:align];
            crate::mac_ffi::CFRelease(align as crate::mac_ffi::CFTypeRef);

            let fg_c: [f64; 4] = [1.0, 1.0, 1.0, 1.0];
            let fg_color = CGColorCreate(cs, fg_c.as_ptr());
            let _: () = msg_send![text, setForegroundColor:fg_color];
            CGColorRelease(fg_color);
            CGColorSpaceRelease(cs);

            let _: () = msg_send![text, setContentsScale:2.0f64];

            let _: () = msg_send![root, addSublayer:bg];
            let _: () = msg_send![root, addSublayer:text];
            let _: () = msg_send![class!(CATransaction), flush];
            let _: () = msg_send![panel, orderFrontRegardless];

            // 1.5秒後に自動消去
            let _: () = msg_send![panel, performSelector:sel!(orderOut:) withObject:nil afterDelay:1.5f64];

            eprintln!("[GridSnap] toast: {}", message);
        }
    }

    /// オーバーレイを非表示にする。メインスレッドから呼ぶこと。
    pub fn hide(&self) {
        unsafe {
            let _: () = msg_send![self.panel, orderOut:nil];
            eprintln!("[GridSnap] overlay hidden");
        }
    }
}

// ──── CoreGraphics color helpers (autorelease-safe) ────

extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> id;
    fn CGColorSpaceRelease(cs: id);
    fn CGColorCreate(space: id, components: *const f64) -> id;
    fn CGColorRelease(color: id);
}