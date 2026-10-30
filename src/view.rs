//! The image view: draws the current image, handles zoom, pan, selection,
//! drag-and-drop and keyboard fallbacks.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBezierPath, NSColor, NSCompositingOperation, NSCursor, NSDragOperation, NSDraggingDestination,
    NSDraggingInfo, NSEvent, NSEventModifierFlags, NSImage, NSPasteboardTypeFileURL, NSResponder,
    NSView,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ZoomMode {
    /// Scale to fit the window, never enlarging.
    Fit,
    /// 1 image pixel = 1 point.
    Actual,
    /// Scale to the window width (may enlarge; scroll vertically).
    FitWidth,
    /// An explicit scale factor.
    Custom(f64),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    None,
    Select { start: NSPoint },
    Pan { last: NSPoint },
}

pub struct ViewIvars {
    image: RefCell<Option<Retained<NSImage>>>,
    zoom: Cell<ZoomMode>,
    /// Pan offset in view points (only meaningful when the image overflows).
    pan: Cell<(f64, f64)>,
    /// Selection in image pixels, top-left origin: (x, y, w, h).
    selection: Cell<Option<(f64, f64, f64, f64)>>,
    drag: Cell<Drag>,
    /// Checkerboard behind transparent images.
    checker: Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "MacViewImageView"]
    #[ivars = ViewIvars]
    pub struct ImageView;

    impl ImageView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            self.draw();
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !crate::app::key_fallback(event) {
                // Let the responder chain beep / handle it.
                let _: () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let flags = event.modifierFlags();
            if flags.contains(NSEventModifierFlags::Option) {
                // Option-drag pans a zoomed-in image.
                self.ivars().drag.set(Drag::Pan { last: p });
                NSCursor::closedHandCursor().set();
            } else if event.clickCount() == 2 {
                // Double-click: toggle fit / 1:1, like IrfanView's left double-click.
                crate::app::toggle_fit_actual();
            } else {
                self.ivars().selection.set(None);
                self.ivars().drag.set(Drag::Select { start: p });
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            match self.ivars().drag.get() {
                Drag::Select { start } => {
                    let rect = self.image_rect();
                    let Some(scale) = self.scale() else { return };
                    let to_img = |q: NSPoint| {
                        (
                            ((q.x - rect.origin.x) / scale).clamp(0.0, self.image_size().0),
                            ((q.y - rect.origin.y) / scale).clamp(0.0, self.image_size().1),
                        )
                    };
                    let (ax, ay) = to_img(start);
                    let (bx, by) = to_img(p);
                    let x = ax.min(bx).floor();
                    let y = ay.min(by).floor();
                    let w = (ax.max(bx).ceil() - x).max(0.0);
                    let h = (ay.max(by).ceil() - y).max(0.0);
                    self.ivars().selection.set(if w >= 1.0 && h >= 1.0 { Some((x, y, w, h)) } else { None });
                    crate::app::update_title();
                    self.setNeedsDisplay(true);
                }
                Drag::Pan { last } => {
                    let (px, py) = self.ivars().pan.get();
                    self.ivars().pan.set((px + (p.x - last.x), py + (p.y - last.y)));
                    self.ivars().drag.set(Drag::Pan { last: p });
                    self.setNeedsDisplay(true);
                }
                Drag::None => {}
            }
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            self.ivars().drag.set(Drag::None);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let flags = event.modifierFlags();
            if flags.contains(NSEventModifierFlags::Command) {
                let dy = event.scrollingDeltaY();
                if dy != 0.0 {
                    let p = self.convertPoint_fromView(event.locationInWindow(), None);
                    let factor = if dy > 0.0 { 1.1 } else { 1.0 / 1.1 };
                    self.zoom_by(factor, Some(p));
                }
            } else {
                let (px, py) = self.ivars().pan.get();
                let (dx, dy) = (event.scrollingDeltaX(), event.scrollingDeltaY());
                self.ivars().pan.set((px + dx, py + dy));
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            self.zoom_by(1.0 + event.magnification(), Some(p));
        }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            let cursor = if self.ivars().image.borrow().is_some() {
                NSCursor::crosshairCursor()
            } else {
                NSCursor::arrowCursor()
            };
            self.addCursorRect_cursor(self.bounds(), &cursor);
        }
    }

    unsafe impl NSObjectProtocol for ImageView {}

    unsafe impl NSDraggingDestination for ImageView {
        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, _sender: &ProtocolObject<dyn NSDraggingInfo>) -> NSDragOperation {
            NSDragOperation::Copy
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &ProtocolObject<dyn NSDraggingInfo>) -> bool {
            // No early `return` here: define_class! converts only the final
            // expression from bool to Bool.
            dropped_file(sender).map(crate::app::open_path).is_some()
        }
    }
);

/// First file URL on a drag pasteboard, as a path.
fn dropped_file(sender: &ProtocolObject<dyn NSDraggingInfo>) -> Option<std::path::PathBuf> {
    let pb = sender.draggingPasteboard();
    let classes = NSArray::from_slice(&[NSURL::class()]);
    let objects = unsafe { pb.readObjectsForClasses_options(&classes, None) }?;
    objects
        .iter()
        .find_map(|obj| obj.downcast_ref::<NSURL>().and_then(|u| u.path()))
        .map(|p| std::path::PathBuf::from(p.to_string()))
}

impl ImageView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            image: RefCell::new(None),
            zoom: Cell::new(ZoomMode::Fit),
            pan: Cell::new((0.0, 0.0)),
            selection: Cell::new(None),
            drag: Cell::new(Drag::None),
            checker: Cell::new(true),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let types = NSArray::from_slice(&[unsafe { NSPasteboardTypeFileURL }]);
        this.registerForDraggedTypes(&types);
        this
    }

    // -- image & state ------------------------------------------------------

    pub fn set_image(&self, image: Option<Retained<NSImage>>, keep_zoom: bool) {
        *self.ivars().image.borrow_mut() = image;
        self.ivars().selection.set(None);
        if !keep_zoom {
            self.ivars().zoom.set(ZoomMode::Fit);
        }
        self.ivars().pan.set((0.0, 0.0));
        if let Some(w) = self.window() {
            w.invalidateCursorRectsForView(self);
        }
        self.setNeedsDisplay(true);
    }

    pub fn image(&self) -> Option<Retained<NSImage>> {
        self.ivars().image.borrow().clone()
    }

    pub fn selection(&self) -> Option<(f64, f64, f64, f64)> {
        self.ivars().selection.get()
    }

    pub fn clear_selection(&self) {
        self.ivars().selection.set(None);
        self.setNeedsDisplay(true);
    }

    pub fn zoom_mode(&self) -> ZoomMode {
        self.ivars().zoom.get()
    }

    pub fn set_zoom_mode(&self, mode: ZoomMode) {
        self.ivars().zoom.set(mode);
        self.ivars().pan.set((0.0, 0.0));
        self.setNeedsDisplay(true);
        crate::app::update_title();
    }

    pub fn toggle_checker(&self) {
        self.ivars().checker.set(!self.ivars().checker.get());
        self.setNeedsDisplay(true);
    }

    /// Current scale factor (image px -> view points), if an image is loaded.
    pub fn scale(&self) -> Option<f64> {
        let (iw, ih) = self.image_size();
        if iw <= 0.0 || ih <= 0.0 {
            return None;
        }
        Some(match self.ivars().zoom.get() {
            ZoomMode::Actual => 1.0,
            ZoomMode::Custom(s) => s,
            ZoomMode::Fit => {
                let b = self.bounds();
                let s = (b.size.width / iw).min(b.size.height / ih);
                s.min(1.0)
            }
            ZoomMode::FitWidth => self.bounds().size.width / iw,
        })
    }

    pub fn zoom_by(&self, factor: f64, anchor: Option<NSPoint>) {
        let Some(old) = self.scale() else { return };
        let new = (old * factor).clamp(0.02, 64.0);
        // Keep the image point under `anchor` fixed.
        let rect = self.image_rect();
        let b = self.bounds();
        let a = anchor.unwrap_or(NSPoint::new(b.size.width / 2.0, b.size.height / 2.0));
        let ix = (a.x - rect.origin.x) / old;
        let iy = (a.y - rect.origin.y) / old;
        self.ivars().zoom.set(ZoomMode::Custom(new));
        // New rect origin without pan, then choose pan so ix,iy lands under a.
        let (iw, ih) = self.image_size();
        let dw = iw * new;
        let dh = ih * new;
        let base_x = if dw <= b.size.width { (b.size.width - dw) / 2.0 } else { 0.0 };
        let base_y = if dh <= b.size.height { (b.size.height - dh) / 2.0 } else { 0.0 };
        let want_x = a.x - ix * new;
        let want_y = a.y - iy * new;
        self.ivars().pan.set((want_x - base_x, want_y - base_y));
        self.setNeedsDisplay(true);
        crate::app::update_title();
    }

    fn image_size(&self) -> (f64, f64) {
        match &*self.ivars().image.borrow() {
            Some(img) => {
                let (w, h) = crate::ops::pixel_size(img);
                (w as f64, h as f64)
            }
            None => (0.0, 0.0),
        }
    }

    #[allow(dead_code)]
    fn image_overflows(&self) -> bool {
        let Some(s) = self.scale() else { return false };
        let (iw, ih) = self.image_size();
        let b = self.bounds();
        iw * s > b.size.width + 0.5 || ih * s > b.size.height + 0.5
    }

    /// Where the image is drawn, in view points (flipped: y down).
    fn image_rect(&self) -> NSRect {
        let b = self.bounds();
        let Some(s) = self.scale() else {
            return NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
        };
        let (iw, ih) = self.image_size();
        let dw = iw * s;
        let dh = ih * s;
        let (px, py) = self.ivars().pan.get();
        let x = if dw <= b.size.width {
            (b.size.width - dw) / 2.0
        } else {
            px.clamp(b.size.width - dw, 0.0)
        };
        let y = if dh <= b.size.height {
            (b.size.height - dh) / 2.0
        } else {
            py.clamp(b.size.height - dh, 0.0)
        };
        // Write the clamped pan back so it doesn't accumulate off-screen.
        self.ivars().pan.set((
            if dw <= b.size.width { 0.0 } else { x },
            if dh <= b.size.height { 0.0 } else { y },
        ));
        NSRect::new(NSPoint::new(x.round(), y.round()), NSSize::new(dw, dh))
    }

    // -- drawing -------------------------------------------------------------

    fn draw(&self) {
        let bounds = self.bounds();
        NSColor::colorWithWhite_alpha(0.12, 1.0).set();
        objc2_app_kit::NSRectFill(bounds);

        let image = self.ivars().image.borrow().clone();
        let Some(image) = image else { return };
        let rect = self.image_rect();

        if self.ivars().checker.get() {
            self.draw_checkerboard(rect);
        }
        unsafe {
            image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                rect,
                NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0)),
                NSCompositingOperation::SourceOver,
                1.0,
                true,
                None,
            );
        }

        if let Some((sx, sy, sw, sh)) = self.ivars().selection.get() {
            let Some(s) = self.scale() else { return };
            let r = NSRect::new(
                NSPoint::new(rect.origin.x + sx * s, rect.origin.y + sy * s),
                NSSize::new(sw * s, sh * s),
            );
            // Dim everything outside the selection a little.
            NSColor::colorWithWhite_alpha(0.0, 0.35).set();
            let path = NSBezierPath::bezierPathWithRect(bounds);
            path.appendBezierPathWithRect(r);
            path.setWindingRule(objc2_app_kit::NSWindingRule::EvenOdd);
            path.fill();
            // White/black double outline so it's visible on anything.
            NSColor::blackColor().set();
            let outer = NSBezierPath::bezierPathWithRect(r);
            outer.setLineWidth(3.0);
            outer.stroke();
            NSColor::whiteColor().set();
            let inner = NSBezierPath::bezierPathWithRect(r);
            inner.setLineWidth(1.0);
            inner.stroke();
        }
    }

    fn draw_checkerboard(&self, rect: NSRect) {
        // Only bother when the image has an alpha channel.
        let has_alpha = self
            .ivars()
            .image
            .borrow()
            .as_ref()
            .and_then(|img| img.representations().iter().next().map(|r| r.hasAlpha()))
            .unwrap_or(false);
        if !has_alpha {
            return;
        }
        let cell = 12.0;
        let light = NSColor::colorWithWhite_alpha(0.85, 1.0);
        let dark = NSColor::colorWithWhite_alpha(0.65, 1.0);
        light.set();
        objc2_app_kit::NSRectFill(rect);
        dark.set();
        let cols = (rect.size.width / cell).ceil() as i64;
        let rows = (rect.size.height / cell).ceil() as i64;
        for row in 0..rows {
            for col in 0..cols {
                if (row + col) % 2 == 0 {
                    continue;
                }
                let x = rect.origin.x + col as f64 * cell;
                let y = rect.origin.y + row as f64 * cell;
                let w = cell.min(rect.origin.x + rect.size.width - x);
                let h = cell.min(rect.origin.y + rect.size.height - y);
                objc2_app_kit::NSRectFill(NSRect::new(NSPoint::new(x, y), NSSize::new(w, h)));
            }
        }
    }
}

/// Helper so other modules can build an NSString quickly.
pub fn ns(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}
