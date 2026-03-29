//! macOS メニューバートレイ（NSStatusItem + NSMenu）。
//! グリッドの columns / rows をプリセットから変更できる。
//! NSApplication の初期化後、run の前に setup() を呼ぶこと。

use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSString};
use objc::declare::ClassDecl;
use objc::runtime::{Class, Object, Sel};
use objc::{class, msg_send, sel, sel_impl};
use std::sync::Once;

use crate::config::Config;

/// プリセット値
const COLUMN_PRESETS: &[u32] = &[4, 6, 8, 12, 16, 20];
const ROW_PRESETS: &[u32] = &[2, 3, 4, 6, 8, 12];

/// メニューアイテムのタグ encode:
/// columns: tag = 1000 + value
/// rows:    tag = 2000 + value
const TAG_COL_BASE: isize = 1000;
const TAG_ROW_BASE: isize = 2000;
const TAG_QUIT: isize = 9999;
const TAG_CAPTURE: isize = 8000;

static REGISTER_CLASS: Once = Once::new();

// ──── Retained status item ────
// NSStatusBar は statusItem を weak reference で保持するため、
// Rust 側で強参照を維持しないと即座に解放される。

#[derive(Clone, Copy)]
struct SendId(id);
unsafe impl Send for SendId {}
unsafe impl Sync for SendId {}

static RETAINED_ITEM: std::sync::Mutex<Option<SendId>> = std::sync::Mutex::new(None);

/// NSStatusItem を作成し、メニューを構築する。
/// EventHookManager::new() の後（NSApplication 初期化済み）、run_loop() の前に呼ぶ。
pub fn setup(config: &Config) {
    register_delegate_class();

    unsafe {
        let pool: id = NSAutoreleasePool::new(nil);

        // ── NSStatusItem ──
        let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
        // NSVariableStatusItemLength = -2
        let item: id = msg_send![status_bar, statusItemWithLength:-2.0f64];
        // statusItemWithLength: は autoreleased を返すので明示 retain
        let _: () = msg_send![item, retain];

        let button: id = msg_send![item, button];
        let title = NSString::alloc(nil).init_str("⊞");
        let _: () = msg_send![button, setTitle:title];

        // ── Delegate（アクションターゲット）──
        let delegate_class = Class::get("GridSnapTrayDelegate").unwrap();
        let delegate: id = msg_send![delegate_class, alloc];
        let delegate: id = msg_send![delegate, init];

        // ── Menu ──
        let menu = build_menu(delegate, config);
        let _: () = msg_send![item, setMenu:menu];

        // delegate を明示 retain（アプリ生存期間全体で必要）
        let _: () = msg_send![delegate, retain];

        // item を静的に保持
        *RETAINED_ITEM.lock().unwrap() = Some(SendId(item));

        let _: () = msg_send![pool, drain];

        eprintln!(
            "[GridSnap] Menu bar tray installed ({}x{})",
            config.grid.columns, config.grid.rows
        );
    }
}

/// メニュー全体を構築する。
unsafe fn build_menu(delegate: id, config: &Config) -> id {
    let menu: id = msg_send![class!(NSMenu), alloc];
    let menu: id = msg_send![menu, init];

    // ── Columns サブメニュー ──
    let col_submenu = build_preset_submenu(
        delegate,
        COLUMN_PRESETS,
        TAG_COL_BASE,
        config.grid.columns,
    );
    let col_item = make_submenu_item("Columns", col_submenu);
    let _: () = msg_send![menu, addItem:col_item];

    // ── Rows サブメニュー ──
    let row_submenu = build_preset_submenu(
        delegate,
        ROW_PRESETS,
        TAG_ROW_BASE,
        config.grid.rows,
    );
    let row_item = make_submenu_item("Rows", row_submenu);
    let _: () = msg_send![menu, addItem:row_item];

    // ── Separator ──
    let sep: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem:sep];

    // ── Capture Position (F0a) ──
    let capture_title = NSString::alloc(nil).init_str("📍 Capture this app's position");
    let capture_key = NSString::alloc(nil).init_str("");
    let capture_item: id = msg_send![class!(NSMenuItem), alloc];
    let capture_item: id = msg_send![capture_item,
        initWithTitle:capture_title
        action:sel!(gridAction:)
        keyEquivalent:capture_key
    ];
    let _: () = msg_send![capture_item, setTarget:delegate];
    let _: () = msg_send![capture_item, setTag:TAG_CAPTURE];
    let _: () = msg_send![menu, addItem:capture_item];

    // ── Separator ──
    let sep2: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem:sep2];

    // ── Quit ──
    let quit_title = NSString::alloc(nil).init_str("Quit GridSnap");
    let empty = NSString::alloc(nil).init_str("q");
    let quit_item: id = msg_send![class!(NSMenuItem), alloc];
    let quit_item: id = msg_send![quit_item,
        initWithTitle:quit_title
        action:sel!(gridAction:)
        keyEquivalent:empty
    ];
    let _: () = msg_send![quit_item, setTarget:delegate];
    let _: () = msg_send![quit_item, setTag:TAG_QUIT];
    let _: () = msg_send![menu, addItem:quit_item];

    menu
}

/// プリセット値のサブメニューを構築する。
unsafe fn build_preset_submenu(
    delegate: id,
    presets: &[u32],
    tag_base: isize,
    current: u32,
) -> id {
    let submenu: id = msg_send![class!(NSMenu), alloc];
    let submenu: id = msg_send![submenu, init];

    for &val in presets {
        let label = format!("{}", val);
        let ns_label = NSString::alloc(nil).init_str(&label);
        let empty = NSString::alloc(nil).init_str("");
        let mi: id = msg_send![class!(NSMenuItem), alloc];
        let mi: id = msg_send![mi,
            initWithTitle:ns_label
            action:sel!(gridAction:)
            keyEquivalent:empty
        ];
        let _: () = msg_send![mi, setTarget:delegate];
        let _: () = msg_send![mi, setTag:(tag_base + val as isize)];
        if val == current {
            let _: () = msg_send![mi, setState:1isize]; // NSOnState
        }
        let _: () = msg_send![submenu, addItem:mi];
    }

    submenu
}

/// サブメニューを持つ親アイテムを作成する。
unsafe fn make_submenu_item(title: &str, submenu: id) -> id {
    let ns_title = NSString::alloc(nil).init_str(title);
    let empty = NSString::alloc(nil).init_str("");
    let item: id = msg_send![class!(NSMenuItem), alloc];
    let item: id = msg_send![item,
        initWithTitle:ns_title
        action:nil
        keyEquivalent:empty
    ];
    let _: () = msg_send![item, setSubmenu:submenu];
    item
}

// ──── ObjC delegate class ────

fn register_delegate_class() {
    REGISTER_CLASS.call_once(|| {
        let superclass = Class::get("NSObject").unwrap();
        let mut decl = ClassDecl::new("GridSnapTrayDelegate", superclass).unwrap();

        unsafe {
            decl.add_method(
                sel!(gridAction:),
                grid_action as extern "C" fn(&Object, Sel, id),
            );
        }

        decl.register();
    });
}

/// メニューアイテムのアクションハンドラ。
extern "C" fn grid_action(_self: &Object, _cmd: Sel, sender: id) {
    unsafe {
        let tag: isize = msg_send![sender, tag];

        // ── Capture Position (F0a) ──
        if tag == TAG_CAPTURE {
            eprintln!("[GridSnap] Capture requested from tray");
            crate::mac_event_hook::capture_frontmost_window();
            return;
        }

        // ── Quit ──
        if tag == TAG_QUIT {
            eprintln!("[GridSnap] Quit requested from tray");
            let app: id = msg_send![class!(NSApplication), sharedApplication];
            let _: () = msg_send![app, terminate:nil];
            return;
        }

        // ── Grid preset ──
        let is_col = tag >= TAG_COL_BASE && tag < TAG_ROW_BASE;
        let value = if is_col {
            (tag - TAG_COL_BASE) as u32
        } else {
            (tag - TAG_ROW_BASE) as u32
        };

        eprintln!(
            "[GridSnap] Tray: {} = {}",
            if is_col { "columns" } else { "rows" },
            value
        );

        // Config を更新 → TOML 保存
        crate::mac_event_hook::update_config(|config| {
            if is_col {
                config.grid.columns = value;
            } else {
                config.grid.rows = value;
            }
        });

        // チェックマーク更新: 同一サブメニュー内の全アイテムを OFF → sender を ON
        let parent_menu: id = msg_send![sender, menu];
        if parent_menu != nil {
            let count: isize = msg_send![parent_menu, numberOfItems];
            for i in 0..count {
                let item: id = msg_send![parent_menu, itemAtIndex:i];
                let _: () = msg_send![item, setState:0isize]; // NSOffState
            }
            let _: () = msg_send![sender, setState:1isize]; // NSOnState
        }
    }
}