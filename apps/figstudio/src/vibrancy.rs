//! macOS: a blurred, translucent window background, like Arc's translucent sidebar.
//!
//! An `NSVisualEffectView` goes behind the window's content view, in the window's frame view.
//! (`window-vibrancy` adds it inside the content view, where it covers wgpu's Metal layer and
//! hides the whole UI.) The window is transparent, so the blur shows wherever egui paints
//! with alpha: the gradient behind the sidebar and around the cards. The cards stay opaque.
//! AppKit turns the blur opaque by itself when "Reduce transparency" is on or the window is
//! inactive.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAutoresizingMaskOptions, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowOrderingMode,
};
use objc2_foundation::NSString;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// The blur view behind the window's content.
#[derive(Debug)]
pub struct Vibrancy {
    view: Retained<NSVisualEffectView>,
    /// The appearance last given to the blur (`None`: the window's own).
    dark: Option<bool>,
}

impl Vibrancy {
    /// Puts the blur behind the window of `cc`. `None` when there is no AppKit view to attach
    /// to (the test harness, a headless run) or when not on the main thread.
    pub fn attach(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let RawWindowHandle::AppKit(handle) = cc.window_handle().ok()?.as_raw() else {
            return None;
        };
        let frame_view = frame_view(handle.ns_view)?;
        let content = frame_view.window()?.contentView()?;
        let blur = NSVisualEffectView::initWithFrame(mtm.alloc(), frame_view.bounds());
        blur.setMaterial(NSVisualEffectMaterial::Sidebar);
        blur.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        blur.setState(NSVisualEffectState::FollowsWindowActiveState);
        blur.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        frame_view.addSubview_positioned_relativeTo(
            &blur,
            NSWindowOrderingMode::Below,
            Some(&content),
        );
        Some(Self {
            view: blur,
            dark: None,
        })
    }

    /// Gives the blur the app's appearance (View → Appearance can differ from the system).
    pub fn set_dark(&mut self, dark: bool) {
        if self.dark == Some(dark) {
            return;
        }
        let name = NSString::from_str(if dark {
            "NSAppearanceNameDarkAqua"
        } else {
            "NSAppearanceNameAqua"
        });
        let appearance = NSAppearance::appearanceNamed(&name);
        self.view.setAppearance(appearance.as_deref());
        self.dark = Some(dark);
    }
}

/// The window's frame view: the superview of eframe's content view `ns_view`.
#[expect(
    unsafe_code,
    reason = "eframe gives its NSView only as a raw pointer, and AppKit's superview getter is unsafe"
)]
fn frame_view(ns_view: std::ptr::NonNull<std::ffi::c_void>) -> Option<Retained<NSView>> {
    // SAFETY: `ns_view` is the live content view of eframe's window: the raw window handle
    // comes from the `CreationContext` and is used before the closure returns, on the main
    // thread (checked by the caller with `MainThreadMarker`). `superview` only reads the view
    // hierarchy and returns a retained view, so nothing here outlives a borrow.
    unsafe { ns_view.cast::<NSView>().as_ref().superview() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_window_there_is_no_blur_and_no_panic() {
        let mut attached = Some(true);
        let _h = egui_kittest::Harness::builder().build_eframe(|cc| {
            attached = Some(Vibrancy::attach(cc).is_some());
            crate::App::new(cc, Vec::new())
        });
        assert_eq!(attached, Some(false));
    }
}
