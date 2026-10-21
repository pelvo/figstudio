//! The Dock menu (macOS): a right-click on the app's Dock icon lists recent files.
//!
//! AppKit asks the app delegate for it (`applicationDockMenu:`), and the delegate is
//! winit's, which has no such method. At start the method is added to that class: it hands
//! back the native menu of a muda submenu (`menu_native.rs`), so its items send the usual
//! menu events.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use muda::ContextMenu as _;
use objc2::MainThreadMarker;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;

/// The Dock menu's `NSMenu`, owned by the submenu kept in `NativeMenu` for the app's life.
static MENU: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// `-[delegate applicationDockMenu:]`: the menu (not retained: AppKit takes it as +0).
extern "C-unwind" fn dock_menu(_this: &AnyObject, _cmd: Sel, _app: &AnyObject) -> *mut c_void {
    MENU.load(Ordering::Relaxed)
}

/// Makes `menu` the Dock menu. Logs and leaves the Dock as it is when it cannot.
pub(crate) fn install(menu: &muda::Submenu) {
    MENU.store(menu.ns_menu(), Ordering::Relaxed);
    let Some(mtm) = MainThreadMarker::new() else {
        tracing::warn!("dock menu: not on the main thread");
        return;
    };
    let Some(delegate) = NSApplication::sharedApplication(mtm).delegate() else {
        tracing::warn!("dock menu: the app has no delegate");
        return;
    };
    let class: &AnyClass = AsRef::<AnyObject>::as_ref(&*delegate).class();
    let types = c"@@:@";
    let method: extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> *mut c_void = dock_menu;
    #[expect(
        unsafe_code,
        reason = "winit's app delegate has no Dock menu method, and adding one is a runtime call"
    )]
    // SAFETY: `class` is the live class of the app's delegate, and this runs once, on the
    // main thread. `method` has the C ABI of an Objective-C method that takes `self`,
    // `_cmd` and one object and returns an object, which is what `types` ("@@:@") declares
    // and what AppKit calls `applicationDockMenu:` with; turning it into the untyped `Imp`
    // only changes its type. It returns a menu that `NativeMenu` keeps alive for the
    // app's life, or nil. `class_addMethod` adds nothing when the class already has the
    // method.
    let added = unsafe {
        let imp = std::mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, &AnyObject) -> *mut c_void,
            Imp,
        >(method);
        objc2::ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationDockMenu:),
            imp,
            types.as_ptr(),
        )
    };
    if !added.as_bool() {
        tracing::warn!("dock menu: the app delegate already has one");
    }
}
