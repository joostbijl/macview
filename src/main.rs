//! MacView — a fast, native macOS image viewer in the spirit of IrfanView.
//!
//! Pure Rust on top of AppKit via the `objc2` bindings. No web view, no
//! cross-platform toolkit: one window, one custom NSView, native menus
//! and panels.

#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]

mod app;
mod dialogs;
mod files;
mod menu;
mod ops;
mod view;

use objc2::MainThreadMarker;
use objc2::DefinedClass;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

fn main() {
    let mtm = MainThreadMarker::new().expect("MacView must start on the main thread");
    let ns_app = NSApplication::sharedApplication(mtm);
    ns_app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let delegate = app::AppDelegate::new(mtm);
    let proto = objc2::runtime::ProtocolObject::from_ref(&*delegate);
    ns_app.setDelegate(Some(proto));

    // A path given on the command line (`macview photo.jpg`) is opened at launch.
    // Double-click / "Open With" from Finder arrives via application:openURLs:.
    if let Some(arg) = std::env::args().nth(1) {
        if !arg.starts_with("-psn_") {
            delegate.ivars().pending.replace(Some(std::path::PathBuf::from(arg)));
        }
    }

    // NSApplication holds its delegate weakly, so keep our strong reference
    // alive for the lifetime of the process.
    std::mem::forget(delegate);
    ns_app.run();
}
