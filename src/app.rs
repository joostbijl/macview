//! Application delegate: owns the window, the image view, the current
//! "document" (file list, index, undo stack) and implements every menu
//! action.

use std::cell::{OnceCell, RefCell};
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSBackingStoreType, NSEvent, NSImage, NSMenuItem,
    NSModalResponseOK, NSOpenPanel, NSPasteboard, NSPasteboardTypePNG, NSPasteboardTypeTIFF,
    NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSURL};

use crate::dialogs;
use crate::files;
use crate::menu;
use crate::ops::{self, Format, Pixels};
use crate::view::{ns, ImageView, ZoomMode};

const UNDO_DEPTH: usize = 12;

#[derive(Default)]
struct Doc {
    /// Image files in the current folder, sorted.
    files: Vec<PathBuf>,
    index: usize,
    /// The file on disk the current image came from (None for a pasted image).
    path: Option<PathBuf>,
    dirty: bool,
    undo: Vec<Retained<NSImage>>,
}

pub struct AppIvars {
    /// A path handed over on the command line, opened once the app is up.
    pub pending: RefCell<Option<PathBuf>>,
    window: OnceCell<Retained<NSWindow>>,
    view: OnceCell<Retained<ImageView>>,
    doc: RefCell<Doc>,
    props: RefCell<Option<Retained<NSWindow>>>,
}

thread_local! {
    static DELEGATE: RefCell<Option<Retained<AppDelegate>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&AppDelegate) -> R) -> Option<R> {
    DELEGATE.with(|d| d.borrow().as_ref().map(|d| f(d)))
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacViewAppDelegate"]
    #[ivars = AppIvars]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _n: &NSNotification) {
            let mtm = self.mtm();
            let app = NSApplication::sharedApplication(mtm);
            self.build_window(mtm);
            menu::install(mtm, &app, self);
            let pending = self.ivars().pending.borrow_mut().take();
            if let Some(p) = pending {
                self.open(p);
            } else {
                self.update_title();
            }
            app.activate();
        }

        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            if let Some(url) = urls.iter().next() {
                if let Some(p) = url.path() {
                    self.open(PathBuf::from(p.to_string()));
                }
            }
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn terminate_after_last_window(&self, _app: &NSApplication) -> bool {
            true
        }
    }

    // Menu validation: everything off while a modal dialog is up (so typing
    // "r" in the rename field doesn't rotate the image), and image actions
    // off when there is nothing loaded.
    impl AppDelegate {
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
            self.menu_item_enabled(item)
        }
    }

    // ---- File ------------------------------------------------------------
    impl AppDelegate {
        #[unsafe(method(openDocument:))]
        fn open_document(&self, _s: Option<&AnyObject>) {
            let panel = NSOpenPanel::openPanel(self.mtm());
            panel.setCanChooseFiles(true);
            panel.setCanChooseDirectories(false);
            panel.setAllowsMultipleSelection(false);
            if panel.runModal() == NSModalResponseOK {
                if let Some(url) = panel.URLs().iter().next() {
                    if let Some(p) = url.path() {
                        self.open(PathBuf::from(p.to_string()));
                    }
                }
            }
        }

        #[unsafe(method(reloadImage:))]
        fn reload_image(&self, _s: Option<&AnyObject>) {
            let path = self.ivars().doc.borrow().path.clone();
            if let Some(p) = path {
                self.open(p);
            }
        }

        #[unsafe(method(saveDocument:))]
        fn save_document(&self, _s: Option<&AnyObject>) {
            let path = self.ivars().doc.borrow().path.clone();
            let fmt = path.as_deref().and_then(|p| Format::from_extension(&files::extension_lower(p)));
            match (path, fmt) {
                (Some(p), Some(fmt)) => self.write_image(&p, fmt, 0.92),
                _ => self.do_save_as(),
            }
        }

        #[unsafe(method(saveDocumentAs:))]
        fn save_document_as(&self, _s: Option<&AnyObject>) {
            self.do_save_as();
        }

        #[unsafe(method(renameFile:))]
        fn rename_file(&self, _s: Option<&AnyObject>) {
            let Some(path) = self.ivars().doc.borrow().path.clone() else { return };
            let Some(new) = dialogs::rename(self.mtm(), &path) else { return };
            if new.exists() {
                dialogs::error(self.mtm(), "Cannot rename", "A file with that name already exists.");
                return;
            }
            match std::fs::rename(&path, &new) {
                Ok(()) => {
                    let mut doc = self.ivars().doc.borrow_mut();
                    let idx = doc.index;
                    if idx < doc.files.len() {
                        doc.files[idx] = new.clone();
                    }
                    doc.path = Some(new);
                    drop(doc);
                    self.update_title();
                }
                Err(e) => dialogs::error(self.mtm(), "Cannot rename", &e.to_string()),
            }
        }

        #[unsafe(method(moveToTrash:))]
        fn move_to_trash(&self, _s: Option<&AnyObject>) {
            let Some(path) = self.ivars().doc.borrow().path.clone() else { return };
            let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
            let urls = NSArray::from_slice(&[&*url]);
            NSWorkspace::sharedWorkspace().recycleURLs_completionHandler(&urls, None);
            // Drop it from the list and show the next one.
            let next = {
                let mut doc = self.ivars().doc.borrow_mut();
                let i = doc.index;
                if i < doc.files.len() {
                    doc.files.remove(i);
                }
                if doc.files.is_empty() {
                    None
                } else {
                    let i = doc.index.min(doc.files.len() - 1);
                    Some(doc.files[i].clone())
                }
            };
            match next {
                Some(p) => self.open_in_list(p),
                None => {
                    self.view().set_image(None, false);
                    *self.ivars().doc.borrow_mut() = Doc::default();
                    self.update_title();
                }
            }
        }

        #[unsafe(method(revealInFinder:))]
        fn reveal_in_finder(&self, _s: Option<&AnyObject>) {
            let Some(path) = self.ivars().doc.borrow().path.clone() else { return };
            let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
            let urls = NSArray::from_slice(&[&*url]);
            NSWorkspace::sharedWorkspace().activateFileViewerSelectingURLs(&urls);
        }

        #[unsafe(method(showProperties:))]
        fn show_properties(&self, _s: Option<&AnyObject>) {
            let Some(path) = self.ivars().doc.borrow().path.clone() else { return };
            let Some(img) = self.view().image() else { return };
            let text = dialogs::properties_text(&path, &img, self.view().scale());
            let title = format!("Properties — {}", files::file_name(&path));
            if let Some(w) = self.ivars().props.borrow().as_ref() {
                w.close();
            }
            let w = dialogs::show_properties(self.mtm(), &title, &text);
            *self.ivars().props.borrow_mut() = Some(w);
        }

        #[unsafe(method(performClose:))]
        fn perform_close(&self, _s: Option<&AnyObject>) {
            if let Some(w) = self.ivars().window.get() {
                w.performClose(None);
            }
        }
    }

    // ---- Edit ------------------------------------------------------------
    impl AppDelegate {
        #[unsafe(method(undoImage:))]
        fn undo_image(&self, _s: Option<&AnyObject>) {
            let prev = self.ivars().doc.borrow_mut().undo.pop();
            if let Some(img) = prev {
                self.view().set_image(Some(img), true);
                let still_dirty = !self.ivars().doc.borrow().undo.is_empty();
                self.ivars().doc.borrow_mut().dirty = still_dirty;
                self.update_title();
            }
        }

        #[unsafe(method(copyImage:))]
        fn copy_image(&self, _s: Option<&AnyObject>) {
            let Some(img) = self.view().image() else { return };
            // Copy the selection if there is one, otherwise the whole image.
            let img = match self.view().selection() {
                Some((x, y, w, h)) => ops::to_image(&ops::crop(&img, x, y, w, h)),
                None => img,
            };
            let pb = NSPasteboard::generalPasteboard();
            pb.clearContents();
            if let Some(png) = ops::encode(&img, Format::Png, 1.0) {
                pb.setData_forType(Some(&png), unsafe { NSPasteboardTypePNG });
            }
            if let Some(tiff) = ops::encode(&img, Format::Tiff, 1.0) {
                pb.setData_forType(Some(&tiff), unsafe { NSPasteboardTypeTIFF });
            }
        }

        #[unsafe(method(pasteImage:))]
        fn paste_image(&self, _s: Option<&AnyObject>) {
            let pb = NSPasteboard::generalPasteboard();
            if !NSImage::canInitWithPasteboard(&pb) {
                return;
            }
            let Some(img) = NSImage::initWithPasteboard(NSImage::alloc(), &pb) else { return };
            let (w, h) = ops::pixel_size(&img);
            img.setSize(NSSize::new(w as f64, h as f64));
            *self.ivars().doc.borrow_mut() = Doc { dirty: true, ..Doc::default() };
            self.view().set_image(Some(img), false);
            self.update_title();
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _s: Option<&AnyObject>) {
            // Selecting the whole image is pointless for crop; treat it as "clear".
            self.view().clear_selection();
            self.update_title();
        }

        #[unsafe(method(clearSelection:))]
        fn clear_selection(&self, _s: Option<&AnyObject>) {
            self.view().clear_selection();
            self.update_title();
        }

        #[unsafe(method(cropToSelection:))]
        fn crop_to_selection(&self, _s: Option<&AnyObject>) {
            let Some((x, y, w, h)) = self.view().selection() else { return };
            self.apply_image(|img| ops::crop(img, x, y, w, h));
        }
    }

    // ---- Image -----------------------------------------------------------
    impl AppDelegate {
        #[unsafe(method(resizeImage:))]
        fn resize_image(&self, _s: Option<&AnyObject>) {
            let Some(img) = self.view().image() else { return };
            let (w, h) = ops::pixel_size(&img);
            let Some(r) = dialogs::resize(self.mtm(), w, h) else { return };
            self.apply_image(|img| ops::resize(img, r.w, r.h, r.quality));
        }

        #[unsafe(method(canvasSize:))]
        fn canvas_size(&self, _s: Option<&AnyObject>) {
            let Some(img) = self.view().image() else { return };
            let (w, h) = ops::pixel_size(&img);
            let Some(r) = dialogs::canvas(self.mtm(), w, h) else { return };
            self.apply_pixels(|px| ops::canvas(px, r.w, r.h, r.fill));
        }

        #[unsafe(method(rotateRight:))]
        fn rotate_right(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::rotate_cw);
        }

        #[unsafe(method(rotateLeft:))]
        fn rotate_left(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::rotate_ccw);
        }

        #[unsafe(method(rotate180:))]
        fn rotate_180(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::rotate_180);
        }

        #[unsafe(method(flipHorizontal:))]
        fn flip_horizontal(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::flip_horizontal);
        }

        #[unsafe(method(flipVertical:))]
        fn flip_vertical(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::flip_vertical);
        }

        #[unsafe(method(grayscale:))]
        fn grayscale(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::grayscale);
        }

        #[unsafe(method(negative:))]
        fn negative(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::negative);
        }

        #[unsafe(method(autoAdjust:))]
        fn auto_adjust(&self, _s: Option<&AnyObject>) {
            self.apply_pixels(ops::auto_adjust);
        }
    }

    // ---- View / Go -------------------------------------------------------
    impl AppDelegate {
        #[unsafe(method(macviewFullScreen:))]
        fn full_screen(&self, _s: Option<&AnyObject>) {
            if let Some(w) = self.ivars().window.get() {
                w.toggleFullScreen(None);
            }
        }

        #[unsafe(method(toggleFitActual:))]
        fn toggle_fit_actual(&self, _s: Option<&AnyObject>) {
            toggle_fit_actual();
        }

        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _s: Option<&AnyObject>) {
            self.view().zoom_by(1.25, None);
        }

        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _s: Option<&AnyObject>) {
            self.view().zoom_by(0.8, None);
        }

        #[unsafe(method(zoomFit:))]
        fn zoom_fit(&self, _s: Option<&AnyObject>) {
            self.view().set_zoom_mode(ZoomMode::Fit);
        }

        #[unsafe(method(zoomActual:))]
        fn zoom_actual(&self, _s: Option<&AnyObject>) {
            self.view().set_zoom_mode(ZoomMode::Actual);
        }

        #[unsafe(method(toggleChecker:))]
        fn toggle_checker(&self, _s: Option<&AnyObject>) {
            self.view().toggle_checker();
        }

        #[unsafe(method(nextImage:))]
        fn next_image(&self, _s: Option<&AnyObject>) {
            self.step(1);
        }

        #[unsafe(method(prevImage:))]
        fn prev_image(&self, _s: Option<&AnyObject>) {
            self.step(-1);
        }

        #[unsafe(method(firstImage:))]
        fn first_image(&self, _s: Option<&AnyObject>) {
            let p = self.ivars().doc.borrow().files.first().cloned();
            if let Some(p) = p {
                self.open_in_list(p);
            }
        }

        #[unsafe(method(lastImage:))]
        fn last_image(&self, _s: Option<&AnyObject>) {
            let p = self.ivars().doc.borrow().files.last().cloned();
            if let Some(p) = p {
                self.open_in_list(p);
            }
        }
    }
);

impl AppDelegate {
    fn menu_item_enabled(&self, item: &NSMenuItem) -> bool {
        let app = NSApplication::sharedApplication(self.mtm());
        if app.modalWindow().is_some() {
            return false;
        }
        let Some(action) = item.action() else { return true };
        let name = action.name().to_string_lossy().into_owned();
        let has_image = self.view().image().is_some();
        let has_file = self.ivars().doc.borrow().path.is_some();
        match name.as_str() {
            "openDocument:" | "pasteImage:" | "macviewFullScreen:" | "performClose:" => true,
            "cropToSelection:" => self.view().selection().is_some(),
            "undoImage:" => !self.ivars().doc.borrow().undo.is_empty(),
            "renameFile:" | "moveToTrash:" | "revealInFinder:" | "reloadImage:" | "showProperties:" => has_file,
            "nextImage:" | "prevImage:" | "firstImage:" | "lastImage:" => self.ivars().doc.borrow().files.len() > 1,
            _ => has_image,
        }
    }

    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(AppIvars {
            pending: RefCell::new(None),
            window: OnceCell::new(),
            view: OnceCell::new(),
            doc: RefCell::new(Doc::default()),
            props: RefCell::new(None),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        DELEGATE.with(|d| *d.borrow_mut() = Some(this.clone()));
        this
    }

    fn view(&self) -> &ImageView {
        self.ivars().view.get().expect("view not built yet")
    }

    fn build_window(&self, mtm: MainThreadMarker) {
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1100.0, 760.0));
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&ns("MacView"));
        window.setMinSize(NSSize::new(320.0, 240.0));
        window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);
        window.setFrameAutosaveName(&ns("MacViewMain"));
        unsafe { window.setReleasedWhenClosed(false) };

        let view = ImageView::new(mtm, frame);
        window.setContentView(Some(&view));
        window.makeFirstResponder(Some(&view));
        window.center();
        window.makeKeyAndOrderFront(None);

        let _ = self.ivars().view.set(view);
        let _ = self.ivars().window.set(window);
    }

    /// Open a file from outside the current folder list (Open…, drag, argv).
    fn open(&self, path: PathBuf) {
        let path = path.canonicalize().unwrap_or(path);
        if path.is_dir() {
            // A folder: open its first image.
            match files::first_image_in(&path) {
                Some(first) => return self.open(first),
                None => {
                    dialogs::error(self.mtm(), "No images", "That folder contains no image files.");
                    return;
                }
            }
        }
        let (list, idx) = files::siblings(&path);
        {
            let mut doc = self.ivars().doc.borrow_mut();
            doc.files = list;
            doc.index = idx;
        }
        self.load(path);
    }

    /// Open a file already in the folder list.
    fn open_in_list(&self, path: PathBuf) {
        {
            let mut doc = self.ivars().doc.borrow_mut();
            if let Some(i) = doc.files.iter().position(|p| *p == path) {
                doc.index = i;
            }
        }
        self.load(path);
    }

    fn load(&self, path: PathBuf) {
        match ops::load(&path) {
            Some(img) => {
                let keep_zoom = matches!(self.view().zoom_mode(), ZoomMode::Actual);
                self.view().set_image(Some(img), keep_zoom);
                let mut doc = self.ivars().doc.borrow_mut();
                doc.path = Some(path);
                doc.dirty = false;
                doc.undo.clear();
            }
            None => {
                dialogs::error(
                    self.mtm(),
                    "Cannot open image",
                    &format!("{} could not be decoded.", files::file_name(&path)),
                );
            }
        }
        self.update_title();
    }

    fn step(&self, delta: isize) {
        let next = {
            let doc = self.ivars().doc.borrow();
            if doc.files.is_empty() {
                return;
            }
            let n = doc.files.len() as isize;
            let i = (doc.index as isize + delta).rem_euclid(n) as usize;
            doc.files[i].clone()
        };
        let dirty = self.ivars().doc.borrow().dirty;
        if dirty && !self.confirm_discard() {
            return;
        }
        self.open_in_list(next);
    }

    fn confirm_discard(&self) -> bool {
        let alert = objc2_app_kit::NSAlert::new(self.mtm());
        alert.setMessageText(&ns("Discard changes?"));
        alert.setInformativeText(&ns("The current image has unsaved changes."));
        alert.addButtonWithTitle(&ns("Discard"));
        alert.addButtonWithTitle(&ns("Cancel"));
        alert.runModal() == objc2_app_kit::NSAlertFirstButtonReturn
    }

    /// Replace the image with `f(image)`, keeping an undo copy.
    fn apply_image(&self, f: impl FnOnce(&NSImage) -> Pixels) {
        let Some(img) = self.view().image() else { return };
        let px = f(&img);
        self.commit(img, px);
    }

    /// Replace the image with `f(pixels)`, keeping an undo copy.
    fn apply_pixels(&self, f: impl FnOnce(&Pixels) -> Pixels) {
        let Some(img) = self.view().image() else { return };
        let px = f(&ops::pixels_of(&img));
        self.commit(img, px);
    }

    fn commit(&self, old: Retained<NSImage>, px: Pixels) {
        let new = ops::to_image(&px);
        {
            let mut doc = self.ivars().doc.borrow_mut();
            doc.undo.push(old);
            if doc.undo.len() > UNDO_DEPTH {
                doc.undo.remove(0);
            }
            doc.dirty = true;
        }
        let keep_zoom = !matches!(self.view().zoom_mode(), ZoomMode::Custom(_));
        self.view().set_image(Some(new), keep_zoom);
        self.update_title();
    }

    fn do_save_as(&self) {
        let current = self.ivars().doc.borrow().path.clone();
        let Some(r) = dialogs::save_as(self.mtm(), current.as_deref()) else { return };
        self.write_image(&r.path, r.format, r.jpeg_quality);
    }

    fn write_image(&self, path: &Path, fmt: Format, quality: f64) {
        let Some(img) = self.view().image() else { return };
        let Some(data) = ops::encode(&img, fmt, quality) else {
            dialogs::error(self.mtm(), "Cannot save", "Encoding the image failed.");
            return;
        };
        let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
        if !data.writeToURL_atomically(&url, true) {
            dialogs::error(self.mtm(), "Cannot save", &format!("Writing {} failed.", path.display()));
            return;
        }
        // Saved to a new file in the same folder: switch to it.
        let same = self.ivars().doc.borrow().path.as_deref() == Some(path);
        if same {
            let mut doc = self.ivars().doc.borrow_mut();
            doc.dirty = false;
            doc.undo.clear();
        } else {
            let (list, idx) = files::siblings(path);
            let mut doc = self.ivars().doc.borrow_mut();
            doc.files = list;
            doc.index = idx;
            doc.path = Some(path.to_path_buf());
            doc.dirty = false;
            doc.undo.clear();
        }
        self.update_title();
    }

    pub fn update_title(&self) {
        let Some(window) = self.ivars().window.get() else { return };
        let doc = self.ivars().doc.borrow();
        let view = self.view();
        let mut parts: Vec<String> = Vec::new();
        match (&doc.path, view.image()) {
            (Some(p), Some(img)) => {
                let (w, h) = ops::pixel_size(&img);
                let n = doc.files.len();
                parts.push(format!("{} ({}/{})", files::file_name(p), doc.index + 1, n.max(1)));
                parts.push(format!("{w} × {h}"));
            }
            (None, Some(img)) => {
                let (w, h) = ops::pixel_size(&img);
                parts.push("Untitled".to_string());
                parts.push(format!("{w} × {h}"));
            }
            _ => parts.push("MacView".to_string()),
        }
        if let Some(s) = view.scale() {
            parts.push(format!("{:.0} %", s * 100.0));
        }
        if let Some((_, _, w, h)) = view.selection() {
            parts.push(format!("selection {} × {}", w as i64, h as i64));
        }
        if doc.dirty {
            parts.push("edited".to_string());
        }
        window.setTitle(&ns(&parts.join(" — ")));
    }
}

// ---------------------------------------------------------------------------
// Entry points used by the view
// ---------------------------------------------------------------------------

pub fn open_path(path: PathBuf) {
    with(|d| d.open(path));
}

pub fn update_title() {
    with(|d| d.update_title());
}

pub fn toggle_fit_actual() {
    with(|d| {
        let v = d.view();
        let mode = match v.zoom_mode() {
            ZoomMode::Fit => ZoomMode::Actual,
            _ => ZoomMode::Fit,
        };
        v.set_zoom_mode(mode);
    });
}

/// Keys that reach the view's keyDown: (i.e. that no menu item claimed).
/// Returns true when handled.
pub fn key_fallback(event: &NSEvent) -> bool {
    let chars = event.charactersIgnoringModifiers().map(|s| s.to_string()).unwrap_or_default();
    let Some(c) = chars.chars().next() else { return false };
    with(|d| {
        let view = d.view();
        match c {
            '\u{1b}' => {
                // Escape: leave full screen first, otherwise drop the selection.
                if let Some(w) = d.ivars().window.get() {
                    if w.styleMask().contains(NSWindowStyleMask::FullScreen) {
                        w.toggleFullScreen(None);
                        return true;
                    }
                }
                view.clear_selection();
                d.update_title();
                true
            }
            ' ' | '\u{F703}' | '\u{F701}' => {
                d.step(1);
                true
            }
            '\u{8}' | '\u{7f}' | '\u{F702}' | '\u{F700}' => {
                d.step(-1);
                true
            }
            '+' | '=' => {
                view.zoom_by(1.25, None);
                true
            }
            '-' => {
                view.zoom_by(0.8, None);
                true
            }
            _ => false,
        }
    })
    .unwrap_or(false)
}

#[allow(dead_code)]
fn _assert_delegate_is_object(d: &AppDelegate) -> &ProtocolObject<dyn NSApplicationDelegate> {
    ProtocolObject::from_ref(d)
}
