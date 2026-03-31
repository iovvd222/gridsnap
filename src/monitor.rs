/// モニター情報の取得。GetDpiForMonitor で物理ピクセル変換を行う。

use anyhow::Result;
use windows_core::BOOL;
use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT},
    Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
        MonitorFromWindow, MONITOR_DEFAULTTONEAREST,
    },
    UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
};

use crate::config::Config;
use crate::grid::Grid;

/// 1モニターの情報
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub handle: isize, // HMONITOR の生値（Send/Sync 用）
    pub device_name: String,
    /// ワークエリア（タスクバーを除いた領域）の物理ピクセル矩形
    pub work_rect: (i32, i32, i32, i32), // (left, top, right, bottom)
    pub dpi: u32,
}

impl MonitorInfo {
    pub fn work_width(&self) -> i32 {
        self.work_rect.2 - self.work_rect.0
    }
    pub fn work_height(&self) -> i32 {
        self.work_rect.3 - self.work_rect.1
    }

    /// このモニターに対応する Grid を返す。
    pub fn to_grid(&self, config: &Config) -> Grid {
        let gc = config.grid_for_monitor(&self.device_name);
        Grid::new(
            self.work_rect.0,
            self.work_rect.1,
            self.work_width(),
            self.work_height(),
            gc.columns,
            gc.rows,
        )
    }
}

/// 全モニターを列挙して MonitorInfo のリストを返す。
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let ptr = &mut monitors as *mut Vec<MonitorInfo> as isize;

    unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_proc),
            LPARAM(ptr),
        );
    }
    eprintln!("[GridSnap] enumerate_monitors: found {} monitors", monitors.len());
    for (i, m) in monitors.iter().enumerate() {
        eprintln!("[GridSnap]   [{}] device={}, work_rect={:?}, dpi={}",
            i, m.device_name, m.work_rect, m.dpi);
    }
    Ok(monitors)
}

unsafe extern "system" fn enum_monitor_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
    if let Some(info) = get_monitor_info(hmonitor) {
        monitors.push(info);
    }
    BOOL(1) // TRUE: 列挙を継続
}

fn get_monitor_info(hmonitor: HMONITOR) -> Option<MonitorInfo> {
    unsafe {
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if !GetMonitorInfoW(hmonitor, &mut info.monitorInfo as *mut _ as *mut _).as_bool() {
            return None;
        }

        let device_name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len())]
        );

        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let dpi = if dpi_x > 0 { dpi_x } else { 96 };

        let wr = info.monitorInfo.rcWork;
        Some(MonitorInfo {
            handle: hmonitor.0 as isize,
            device_name,
            work_rect: (wr.left, wr.top, wr.right, wr.bottom),
            dpi,
        })
    }
}

/// ウィンドウが所属するモニターの MonitorInfo を返す。
pub fn monitor_for_window(hwnd: HWND) -> Option<MonitorInfo> {
    unsafe {
        let hmonitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        get_monitor_info(hmonitor)
    }
}