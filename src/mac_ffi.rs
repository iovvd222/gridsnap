//! Raw FFI bindings for macOS Accessibility, CoreGraphics, CoreFoundation.
//! No external crate dependencies — links directly against system frameworks.

#![allow(non_upper_case_globals, non_camel_case_types, dead_code)]

use std::os::raw::c_void;

// ──── Core CF types ────

pub type CFTypeRef = *const c_void;
pub type CFStringRef = *const c_void;
pub type CFArrayRef = *const c_void;
pub type CFDictionaryRef = *const c_void;
pub type CFAllocatorRef = *const c_void;
pub type CFRunLoopRef = *const c_void;
pub type CFRunLoopSourceRef = *const c_void;
pub type CFRunLoopTimerRef = *const c_void;
pub type CFRunLoopMode = CFStringRef;
pub type CFIndex = isize;
pub type CFAbsoluteTime = f64;
pub type CFTimeInterval = f64;
pub type CFOptionFlags = u64;
pub type Boolean = u8;

// ──── Geometry ────

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

// ──── Display ────

pub type CGDirectDisplayID = u32;

// ──── Accessibility types ────

pub type AXUIElementRef = CFTypeRef;
pub type AXObserverRef = CFTypeRef;
pub type AXError = i32;

pub const kAXErrorSuccess: AXError = 0;
pub const kAXErrorNotificationAlreadyRegistered: AXError = -25076;

pub const kAXValueTypeCGPoint: u32 = 1;
pub const kAXValueTypeCGSize: u32 = 2;

pub type AXObserverCallback = unsafe extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
);

// ──── Window list ────

pub const kCGWindowListOptionOnScreenOnly: u32 = 1;
pub const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
pub const kCGNullWindowID: u32 = 0;

// ──── Event source / key codes ────

pub const kCGEventSourceStateCombinedSessionState: u32 = 0;
pub const kVK_Shift: u16 = 56;
pub const kVK_RightShift: u16 = 60;

// ──── CGEventTap ────

pub type CGEventRef = *const c_void;
pub type CGEventMask = u64;
pub type CFMachPortRef = *const c_void;

pub const kCGSessionEventTap: u32 = 1;
pub const kCGHeadInsertEventTap: u32 = 0;
pub const kCGEventTapOptionListenOnly: u32 = 1;

pub const kCGEventLeftMouseDown: u32 = 1;
pub const kCGEventLeftMouseUp: u32 = 2;
pub const kCGEventLeftMouseDragged: u32 = 6;

pub type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: *const c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

// ──── CFNumber ────

pub const kCFNumberSInt32Type: i32 = 3;
pub const kCFStringEncodingUTF8: u32 = 0x08000100;

// ──── Timer callback ────

pub type CFRunLoopTimerCallBack =
    extern "C" fn(timer: CFRunLoopTimerRef, info: *mut c_void);

#[repr(C)]
pub struct CFRunLoopTimerContext {
    pub version: CFIndex,
    pub info: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub release: Option<unsafe extern "C" fn(*const c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const c_void) -> CFStringRef>,
}

// ──── CoreFoundation ────

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFAllocatorDefault: CFAllocatorRef;
    pub static kCFRunLoopDefaultMode: CFRunLoopMode;

    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;

    pub fn CFStringCreateWithCString(
        alloc: CFAllocatorRef,
        c_str: *const i8,
        encoding: u32,
    ) -> CFStringRef;
    pub fn CFStringGetCStringPtr(s: CFStringRef, encoding: u32) -> *const i8;
    pub fn CFStringGetLength(s: CFStringRef) -> CFIndex;
    pub fn CFStringGetCString(
        s: CFStringRef,
        buf: *mut i8,
        buf_size: CFIndex,
        encoding: u32,
    ) -> Boolean;

    pub fn CFArrayGetCount(arr: CFArrayRef) -> CFIndex;
    pub fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: CFIndex) -> *const c_void;

    pub fn CFDictionaryGetValue(
        dict: CFDictionaryRef,
        key: *const c_void,
    ) -> *const c_void;

    pub fn CFNumberGetValue(
        num: CFTypeRef,
        the_type: i32,
        out: *mut c_void,
    ) -> Boolean;

    pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    pub fn CFRunLoopRun();
    pub fn CFRunLoopAddSource(
        rl: CFRunLoopRef,
        source: CFRunLoopSourceRef,
        mode: CFRunLoopMode,
    );
    pub fn CFRunLoopTimerCreate(
        allocator: CFAllocatorRef,
        fire_date: CFAbsoluteTime,
        interval: CFTimeInterval,
        flags: CFOptionFlags,
        order: CFIndex,
        callout: CFRunLoopTimerCallBack,
        context: *mut CFRunLoopTimerContext,
    ) -> CFRunLoopTimerRef;
    pub fn CFRunLoopAddTimer(
        rl: CFRunLoopRef,
        timer: CFRunLoopTimerRef,
        mode: CFRunLoopMode,
    );
    pub fn CFAbsoluteTimeGetCurrent() -> CFAbsoluteTime;
}

// ──── CoreGraphics ────

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGGetActiveDisplayList(
        max: u32,
        displays: *mut CGDirectDisplayID,
        count: *mut u32,
    ) -> i32;
    pub fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    pub fn CGMainDisplayID() -> CGDirectDisplayID;
    pub fn CGWindowListCopyWindowInfo(
        option: u32,
        relative: u32,
    ) -> CFArrayRef;
    pub fn CGEventSourceKeyState(state_id: u32, key: u16) -> bool;
    pub fn CGEventSourceButtonState(state_id: u32, button: u32) -> bool;

    pub static kCGWindowOwnerPID: CFStringRef;
    pub static kCGWindowLayer: CFStringRef;

    // ──── CGEventTap ────
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: CGEventMask,
        callback: CGEventTapCallBack,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

// ──── CoreFoundation (CFMachPort) ────

extern "C" {
    pub fn CFMachPortCreateRunLoopSource(
        allocator: CFAllocatorRef,
        port: CFMachPortRef,
        order: CFIndex,
    ) -> CFRunLoopSourceRef;
}

// ──── Accessibility ────

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attr: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attr: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    pub fn AXIsProcessTrusted() -> bool;
    pub fn AXObserverCreate(
        pid: i32,
        callback: AXObserverCallback,
        out: *mut AXObserverRef,
    ) -> AXError;
    pub fn AXObserverAddNotification(
        observer: AXObserverRef,
        element: AXUIElementRef,
        notification: CFStringRef,
        refcon: *mut c_void,
    ) -> AXError;
    pub fn AXObserverGetRunLoopSource(
        observer: AXObserverRef,
    ) -> CFRunLoopSourceRef;
    pub fn AXUIElementGetPid(
        element: AXUIElementRef,
        pid: *mut i32,
    ) -> AXError;
    pub fn AXValueCreate(
        vtype: u32,
        value: *const c_void,
    ) -> CFTypeRef;
    pub fn AXValueGetValue(
        value: CFTypeRef,
        vtype: u32,
        out: *mut c_void,
    ) -> Boolean;
}

// ──── IOKit (display name) ────

pub type io_iterator_t = u32;
pub type io_object_t = u32;
pub type io_service_t = u32;
pub type kern_return_t = i32;
pub type IOOptionBits = u32;

pub const KERN_SUCCESS: kern_return_t = 0;
pub const kIODisplayOnlyPreferredName: IOOptionBits = 0x00000200;
pub const kNilOptions: IOOptionBits = 0;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub fn IOServiceMatching(name: *const i8) -> CFDictionaryRef;
    pub fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: CFDictionaryRef,
        existing: *mut io_iterator_t,
    ) -> kern_return_t;
    pub fn IOIteratorNext(iterator: io_iterator_t) -> io_object_t;
    pub fn IOObjectRelease(object: io_object_t) -> kern_return_t;
    pub fn IODisplayCreateInfoDictionary(
        display: io_service_t,
        options: IOOptionBits,
    ) -> CFDictionaryRef;
    pub fn IORegistryEntryCreateCFProperties(
        entry: io_service_t,
        properties: *mut CFDictionaryRef,
        allocator: CFAllocatorRef,
        options: IOOptionBits,
    ) -> kern_return_t;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGDisplayVendorNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayModelNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayIsBuiltin(display: CGDirectDisplayID) -> Boolean;
}

// ──---- Helper: CFDictionary typed access ------

/// CFDictionary から CFString キーで値を取得する。
pub unsafe fn cf_dict_get_value(dict: CFDictionaryRef, key: &str) -> CFTypeRef {
    let cf_key = cf_str(key);
    let val = CFDictionaryGetValue(dict, cf_key as *const c_void);
    CFRelease(cf_key);
    val
}

// ──── Helper functions ────

/// CFString を作成する。呼び出し元が CFRelease する責任を持つ。
pub fn cf_str(s: &str) -> CFStringRef {
    let c_str = std::ffi::CString::new(s).unwrap();
    unsafe {
        CFStringCreateWithCString(
            kCFAllocatorDefault,
            c_str.as_ptr(),
            kCFStringEncodingUTF8,
        )
    }
}

/// CFStringRef → Rust String
pub fn cf_string_to_string(s: CFStringRef) -> String {
    if s.is_null() {
        return String::new();
    }
    unsafe {
        let ptr = CFStringGetCStringPtr(s, kCFStringEncodingUTF8);
        if !ptr.is_null() {
            return std::ffi::CStr::from_ptr(ptr)
                .to_string_lossy()
                .into_owned();
        }
        let len = CFStringGetLength(s);
        let buf_size = len * 4 + 1;
        let mut buf = vec![0i8; buf_size as usize];
        if CFStringGetCString(s, buf.as_mut_ptr(), buf_size, kCFStringEncodingUTF8) != 0 {
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_string_lossy()
                .into_owned()
        } else {
            String::new()
        }
    }
}