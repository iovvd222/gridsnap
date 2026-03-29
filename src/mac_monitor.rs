//! macOS ディスプレイ列挙 (CoreGraphics + NSScreen).
//! Windows の monitor.rs に対応する macOS 実装。
//!
//! CGDisplayBounds → フルスクリーン領域（モニター判定用）
//! NSScreen.visibleFrame → メニューバー/Dock 除外領域（グリッド用）

use crate::config::Config;
use crate::grid::Grid;
use crate::mac_ffi::*;

use cocoa::base::id;
use cocoa::foundation::NSRect;
use objc::{class, msg_send, sel, sel_impl};

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub display_id: CGDirectDisplayID,
    /// フルスクリーン領域（CG 座標）。monitor_for_point 判定用。
    pub bounds: CGRect,
    /// メニューバー・Dock を除いた作業領域（CG 座標）。グリッド用。
    pub visible_bounds: CGRect,
    pub is_main: bool,
}

impl MonitorInfo {
    pub fn work_width(&self) -> i32 {
        self.visible_bounds.size.width as i32
    }

    pub fn work_height(&self) -> i32 {
        self.visible_bounds.size.height as i32
    }

    pub fn origin_x(&self) -> i32 {
        self.visible_bounds.origin.x as i32
    }

    pub fn origin_y(&self) -> i32 {
        self.visible_bounds.origin.y as i32
    }

    /// Config からグリッド設定を取得して Grid を生成する。
    /// visibleFrame ベースなのでメニューバー領域は含まない。
    pub fn to_grid(&self, config: &Config) -> Grid {
        let key = format!("{}", self.display_id);
        let gc = config.grid_for_monitor(&key);
        Grid::new(
            self.origin_x(),
            self.origin_y(),
            self.work_width(),
            self.work_height(),
            gc.columns,
            gc.rows,
        )
    }

    /// 指定座標がこのモニター内に含まれるか判定する（フル領域で判定）。
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.bounds.origin.x
            && x < self.bounds.origin.x + self.bounds.size.width
            && y >= self.bounds.origin.y
            && y < self.bounds.origin.y + self.bounds.size.height
    }
}

/// アクティブな全ディスプレイを列挙する。
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut count: u32 = 0;
    unsafe {
        if CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) != 0
            || count == 0
        {
            return vec![];
        }
    }
    let mut ids = vec![0u32; count as usize];
    unsafe {
        if CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut count) != 0 {
            return vec![];
        }
    }
    let main_id = unsafe { CGMainDisplayID() };
    let main_bounds = unsafe { CGDisplayBounds(main_id) };
    let main_h = main_bounds.size.height;

    // NSScreen から visibleFrame を CG 座標で取得
    let visible_map = unsafe { collect_nsscreen_visible_frames(main_h) };

    ids.iter()
        .map(|&id| {
            let bounds = unsafe { CGDisplayBounds(id) };
            // bounds と NSScreen を origin.x + size で照合
            let visible = visible_map
                .iter()
                .find(|(cg_b, _)| {
                    (cg_b.origin.x - bounds.origin.x).abs() < 1.0
                        && (cg_b.size.width - bounds.size.width).abs() < 1.0
                        && (cg_b.size.height - bounds.size.height).abs() < 1.0
                })
                .map(|(_, vb)| *vb)
                .unwrap_or(bounds); // フォールバック: フル領域

            MonitorInfo {
                display_id: id,
                bounds,
                visible_bounds: visible,
                is_main: id == main_id,
            }
        })
        .collect()
}

/// NSScreen の frame / visibleFrame を CG 座標系に変換して収集する。
/// 戻り値: Vec<(cg_bounds, cg_visible_bounds)>
unsafe fn collect_nsscreen_visible_frames(main_h: f64) -> Vec<(CGRect, CGRect)> {
    let screens: id = msg_send![class!(NSScreen), screens];
    let count: usize = msg_send![screens, count];
    let mut result = Vec::with_capacity(count);

    for i in 0..count {
        let screen: id = msg_send![screens, objectAtIndex:i];
        let frame: NSRect = msg_send![screen, frame];
        let vf: NSRect = msg_send![screen, visibleFrame];

        // NS 座標 (bottom-up) → CG 座標 (top-down)
        let cg_bounds = CGRect {
            origin: CGPoint {
                x: frame.origin.x,
                y: main_h - frame.origin.y - frame.size.height,
            },
            size: CGSize {
                width: frame.size.width,
                height: frame.size.height,
            },
        };
        let cg_visible = CGRect {
            origin: CGPoint {
                x: vf.origin.x,
                y: main_h - vf.origin.y - vf.size.height,
            },
            size: CGSize {
                width: vf.size.width,
                height: vf.size.height,
            },
        };
        result.push((cg_bounds, cg_visible));
    }
    result
}

/// 指定スクリーン座標を含むモニターを返す。
pub fn monitor_for_point(x: f64, y: f64) -> Option<MonitorInfo> {
    enumerate_monitors().into_iter().find(|m| m.contains(x, y))
}

/// 指定座標に最も近いモニターを返す（clamp ベースの最短距離）。
/// monitor_for_point が None のときのフォールバック用。
pub fn monitor_nearest_point(x: f64, y: f64) -> Option<MonitorInfo> {
    let monitors = enumerate_monitors();
    monitors
        .into_iter()
        .min_by(|a, b| {
            let da = rect_distance_sq(&a.bounds, x, y);
            let db = rect_distance_sq(&b.bounds, x, y);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// CGDirectDisplayID からディスプレイのモデル名を取得する。
/// IOKit の IODisplayConnect を列挙し、vendor/model 番号でマッチさせて
/// EDID の kDisplayProductName を返す。
/// 取得できない場合は "Built-in Display" または "External-<id>" を返す。
pub fn display_name(display_id: CGDirectDisplayID) -> String {
    unsafe {
        let target_vendor = CGDisplayVendorNumber(display_id);
        let target_model = CGDisplayModelNumber(display_id);

        let matching = IOServiceMatching(b"IODisplayConnect\0".as_ptr() as *const i8);
        if matching.is_null() {
            return display_name_fallback(display_id);
        }

        let mut iterator: io_iterator_t = 0;
        if IOServiceGetMatchingServices(0, matching, &mut iterator) != KERN_SUCCESS {
            return display_name_fallback(display_id);
        }

        loop {
            let service = IOIteratorNext(iterator);
            if service == 0 {
                break;
            }

            let info = IODisplayCreateInfoDictionary(service, kIODisplayOnlyPreferredName);
            IOObjectRelease(service);
            if info.is_null() {
                continue;
            }

            // vendor / product 番号を照合
            let vendor_ref = cf_dict_get_value(info, "DisplayVendorID");
            let product_ref = cf_dict_get_value(info, "DisplayProductID");

            let mut vendor: u32 = 0;
            let mut product: u32 = 0;
            if !vendor_ref.is_null() {
                CFNumberGetValue(
                    vendor_ref,
                    kCFNumberSInt32Type,
                    &mut vendor as *mut _ as *mut std::os::raw::c_void,
                );
            }
            if !product_ref.is_null() {
                CFNumberGetValue(
                    product_ref,
                    kCFNumberSInt32Type,
                    &mut product as *mut _ as *mut std::os::raw::c_void,
                );
            }

            if vendor != target_vendor || product != target_model {
                CFRelease(info as CFTypeRef);
                continue;
            }

            // kDisplayProductName は CFDictionary<locale, CFString>
            let names_ref = cf_dict_get_value(info, "DisplayProductName");
            if !names_ref.is_null() {
                let names_dict = names_ref as CFDictionaryRef;
                // "en_US" を優先、なければ最初のエントリ
                let en_key = cf_str("en_US");
                let mut name_cf = CFDictionaryGetValue(names_dict, en_key as *const std::os::raw::c_void);
                CFRelease(en_key);
                if name_cf.is_null() {
                    // 最初のエントリを取得: CFArrayで全valuesを取って先頭
                    // 簡易実装: よく使われるロケールを試す
                    for locale in &["ja_JP", "en", "ja"] {
                        let lk = cf_str(locale);
                        name_cf = CFDictionaryGetValue(names_dict, lk as *const std::os::raw::c_void);
                        CFRelease(lk);
                        if !name_cf.is_null() {
                            break;
                        }
                    }
                }
                if !name_cf.is_null() {
                    let name = cf_string_to_string(name_cf as CFStringRef);
                    CFRelease(info as CFTypeRef);
                    IOObjectRelease(iterator);
                    if !name.is_empty() {
                        return name;
                    }
                }
            }

            CFRelease(info as CFTypeRef);
        }

        IOObjectRelease(iterator);
        display_name_fallback(display_id)
    }
}

/// モデル名が取れなかった場合のフォールバック。
fn display_name_fallback(display_id: CGDirectDisplayID) -> String {
    unsafe {
        if CGDisplayIsBuiltin(display_id) != 0 {
            "Built-in Display".to_string()
        } else {
            format!("External-{}", display_id)
        }
    }
}

/// 点 (px, py) と矩形の最短距離の二乗を返す。
fn rect_distance_sq(r: &CGRect, px: f64, py: f64) -> f64 {
    let cx = px.clamp(r.origin.x, r.origin.x + r.size.width);
    let cy = py.clamp(r.origin.y, r.origin.y + r.size.height);
    (px - cx) * (px - cx) + (py - cy) * (py - cy)
}