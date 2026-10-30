//! Modal dialogs: resize, canvas size, save-as options, rename, properties.
//! All built in code with plain frames (AppKit accessory views are not
//! flipped: y grows upwards).

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSBackingStoreType, NSBitmapImageRep, NSButton,
    NSButtonType, NSControlStateValueOn, NSFont, NSFontWeightRegular, NSImage,
    NSImageEXIFData, NSModalResponseOK, NSPopUpButton, NSSavePanel, NSSlider,
    NSTextField, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString, NSURL};

use crate::files;
use crate::ops::{self, Format, Quality};
use crate::view::ns;

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn label(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&ns(text), mtm);
    l.setFrame(frame);
    l
}

fn field(mtm: MainThreadMarker, text: &str, frame: NSRect) -> Retained<NSTextField> {
    let f = NSTextField::textFieldWithString(&ns(text), mtm);
    f.setFrame(frame);
    f
}

fn popup(mtm: MainThreadMarker, titles: &[&str], frame: NSRect, selected: usize) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), frame, false);
    let arr: Vec<Retained<NSString>> = titles.iter().map(|t| ns(t)).collect();
    p.addItemsWithTitles(&NSArray::from_retained_slice(&arr));
    p.selectItemAtIndex(selected as isize);
    p
}

fn checkbox(mtm: MainThreadMarker, title: &str, frame: NSRect, on: bool) -> Retained<NSButton> {
    let b = NSButton::initWithFrame(NSButton::alloc(mtm), frame);
    b.setButtonType(NSButtonType::Switch);
    b.setTitle(&ns(title));
    if on {
        b.setState(NSControlStateValueOn);
    }
    b
}

fn parse_num(f: &NSTextField) -> Option<f64> {
    let s = f.stringValue().to_string();
    let s = s.trim().replace(',', ".");
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

pub fn error(mtm: MainThreadMarker, title: &str, detail: &str) {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns(title));
    alert.setInformativeText(&ns(detail));
    alert.addButtonWithTitle(&ns("OK"));
    alert.runModal();
}

// ---------------------------------------------------------------------------
// Resize / resample
// ---------------------------------------------------------------------------

pub struct ResizeResult {
    pub w: usize,
    pub h: usize,
    pub quality: Quality,
}

pub fn resize(mtm: MainThreadMarker, cur_w: usize, cur_h: usize) -> Option<ResizeResult> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns("Resize / Resample"));
    alert.setInformativeText(&ns(&format!("Current size: {cur_w} × {cur_h} px")));
    alert.addButtonWithTitle(&ns("Resize"));
    alert.addButtonWithTitle(&ns("Cancel"));

    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 340.0, 170.0));

    // Row 1: width / height
    view.addSubview(&label(mtm, "Width:", rect(0.0, 140.0, 60.0, 22.0)));
    let w_field = field(mtm, &cur_w.to_string(), rect(65.0, 138.0, 90.0, 24.0));
    view.addSubview(&w_field);
    view.addSubview(&label(mtm, "Height:", rect(170.0, 140.0, 60.0, 22.0)));
    let h_field = field(mtm, &cur_h.to_string(), rect(230.0, 138.0, 90.0, 24.0));
    view.addSubview(&h_field);

    // Row 2: unit
    view.addSubview(&label(mtm, "Unit:", rect(0.0, 104.0, 60.0, 22.0)));
    let unit = popup(mtm, &["pixels", "percent"], rect(65.0, 100.0, 120.0, 26.0), 0);
    view.addSubview(&unit);

    // Row 3: presets (long side)
    view.addSubview(&label(mtm, "Preset:", rect(0.0, 70.0, 60.0, 22.0)));
    let preset = popup(
        mtm,
        &["— none —", "Long side 640", "Long side 1024", "Long side 1600", "Long side 2048", "Long side 4096", "Half size", "Double size"],
        rect(65.0, 66.0, 200.0, 26.0),
        0,
    );
    view.addSubview(&preset);

    // Row 4: keep aspect + quality
    let keep = checkbox(mtm, "Keep aspect ratio", rect(0.0, 34.0, 160.0, 22.0), true);
    view.addSubview(&keep);
    view.addSubview(&label(mtm, "Resample:", rect(0.0, 4.0, 70.0, 22.0)));
    let quality = popup(mtm, &["High (Lanczos-like)", "Medium", "Low", "Nearest neighbour"], rect(75.0, 0.0, 180.0, 26.0), 0);
    view.addSubview(&quality);

    alert.setAccessoryView(Some(&view));
    alert.window().setInitialFirstResponder(Some(&w_field));

    if alert.runModal() != NSAlertFirstButtonReturn {
        return None;
    }

    let q = match quality.indexOfSelectedItem() {
        0 => Quality::High,
        1 => Quality::Medium,
        2 => Quality::Low,
        _ => Quality::Nearest,
    };
    let aspect = cur_w as f64 / cur_h.max(1) as f64;

    // Presets win over the fields.
    let preset_idx = preset.indexOfSelectedItem();
    if preset_idx > 0 {
        let (w, h) = match preset_idx {
            1..=5 => {
                let long = [640.0, 1024.0, 1600.0, 2048.0, 4096.0][(preset_idx - 1) as usize];
                if cur_w >= cur_h {
                    (long, long / aspect)
                } else {
                    (long * aspect, long)
                }
            }
            6 => (cur_w as f64 / 2.0, cur_h as f64 / 2.0),
            _ => (cur_w as f64 * 2.0, cur_h as f64 * 2.0),
        };
        return Some(ResizeResult { w: w.round().max(1.0) as usize, h: h.round().max(1.0) as usize, quality: q });
    }

    let mut w = parse_num(&w_field);
    let mut h = parse_num(&h_field);
    if unit.indexOfSelectedItem() == 1 {
        w = w.map(|p| cur_w as f64 * p / 100.0);
        h = h.map(|p| cur_h as f64 * p / 100.0);
    }
    let keep_aspect = keep.state() == NSControlStateValueOn;
    let (w, h) = match (w, h) {
        (Some(w), Some(h)) if keep_aspect => {
            // Whichever field the user changed drives the other.
            let w_changed = (w - cur_w as f64).abs() > 0.5;
            let h_changed = (h - cur_h as f64).abs() > 0.5;
            if h_changed && !w_changed {
                (h * aspect, h)
            } else {
                (w, w / aspect)
            }
        }
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w / aspect),
        (None, Some(h)) => (h * aspect, h),
        (None, None) => {
            error(mtm, "Invalid size", "Enter a width and/or height.");
            return None;
        }
    };
    if w < 1.0 || h < 1.0 || w > 30000.0 || h > 30000.0 {
        error(mtm, "Invalid size", "Dimensions must be between 1 and 30000 pixels.");
        return None;
    }
    Some(ResizeResult { w: w.round() as usize, h: h.round() as usize, quality: q })
}

// ---------------------------------------------------------------------------
// Canvas size
// ---------------------------------------------------------------------------

pub struct CanvasResult {
    pub w: usize,
    pub h: usize,
    pub fill: [u8; 4],
}

pub fn canvas(mtm: MainThreadMarker, cur_w: usize, cur_h: usize) -> Option<CanvasResult> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns("Canvas Size"));
    alert.setInformativeText(&ns(&format!(
        "Current size: {cur_w} × {cur_h} px. The image stays centred; a larger canvas adds a border, a smaller one crops."
    )));
    alert.addButtonWithTitle(&ns("Apply"));
    alert.addButtonWithTitle(&ns("Cancel"));

    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 340.0, 70.0));
    view.addSubview(&label(mtm, "Width:", rect(0.0, 42.0, 60.0, 22.0)));
    let w_field = field(mtm, &cur_w.to_string(), rect(65.0, 40.0, 90.0, 24.0));
    view.addSubview(&w_field);
    view.addSubview(&label(mtm, "Height:", rect(170.0, 42.0, 60.0, 22.0)));
    let h_field = field(mtm, &cur_h.to_string(), rect(230.0, 40.0, 90.0, 24.0));
    view.addSubview(&h_field);
    view.addSubview(&label(mtm, "Border:", rect(0.0, 6.0, 60.0, 22.0)));
    let fill = popup(mtm, &["White", "Black", "Gray", "Transparent"], rect(65.0, 2.0, 140.0, 26.0), 0);
    view.addSubview(&fill);
    alert.setAccessoryView(Some(&view));
    alert.window().setInitialFirstResponder(Some(&w_field));

    if alert.runModal() != NSAlertFirstButtonReturn {
        return None;
    }
    let (Some(w), Some(h)) = (parse_num(&w_field), parse_num(&h_field)) else {
        error(mtm, "Invalid size", "Enter a width and height.");
        return None;
    };
    if w < 1.0 || h < 1.0 || w > 30000.0 || h > 30000.0 {
        error(mtm, "Invalid size", "Dimensions must be between 1 and 30000 pixels.");
        return None;
    }
    let fill = match fill.indexOfSelectedItem() {
        0 => [255, 255, 255, 255],
        1 => [0, 0, 0, 255],
        2 => [128, 128, 128, 255],
        _ => [0, 0, 0, 0],
    };
    Some(CanvasResult { w: w.round() as usize, h: h.round() as usize, fill })
}

// ---------------------------------------------------------------------------
// Save As
// ---------------------------------------------------------------------------

pub struct SaveResult {
    pub path: PathBuf,
    pub format: Format,
    pub jpeg_quality: f64,
}

pub fn save_as(mtm: MainThreadMarker, current: Option<&Path>) -> Option<SaveResult> {
    let panel = NSSavePanel::savePanel(mtm);
    panel.setTitle(Some(&ns("Save Image As")));
    panel.setCanCreateDirectories(true);
    panel.setExtensionHidden(false);

    let cur_fmt = current
        .map(|p| files::extension_lower(p))
        .and_then(|e| Format::from_extension(&e))
        .unwrap_or(Format::Png);
    let fmt_index = Format::ALL.iter().position(|f| *f == cur_fmt).unwrap_or(1);

    if let Some(p) = current {
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
        panel.setNameFieldStringValue(&ns(&format!("{stem}.{}", cur_fmt.extension())));
        if let Some(dir) = p.parent() {
            panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&ns(&dir.to_string_lossy()))));
        }
    } else {
        panel.setNameFieldStringValue(&ns("image.png"));
    }

    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 420.0, 64.0));
    view.addSubview(&label(mtm, "Format:", rect(10.0, 36.0, 60.0, 22.0)));
    let labels: Vec<&str> = Format::ALL.iter().map(|f| f.label()).collect();
    let fmt = popup(mtm, &labels, rect(75.0, 32.0, 120.0, 26.0), fmt_index);
    view.addSubview(&fmt);
    view.addSubview(&label(mtm, "JPEG quality:", rect(10.0, 6.0, 95.0, 22.0)));
    let slider = unsafe { NSSlider::sliderWithValue_minValue_maxValue_target_action(90.0, 10.0, 100.0, None, None, mtm) };
    slider.setFrame(rect(110.0, 4.0, 220.0, 24.0));
    view.addSubview(&slider);
    let q_label = label(mtm, "90", rect(340.0, 6.0, 50.0, 22.0));
    view.addSubview(&q_label);
    panel.setAccessoryView(Some(&view));

    if panel.runModal() != NSModalResponseOK {
        return None;
    }
    let url = panel.URL()?;
    let mut path = PathBuf::from(url.path()?.to_string());
    let format = Format::ALL[fmt.indexOfSelectedItem().clamp(0, Format::ALL.len() as isize - 1) as usize];
    // Make the extension agree with the chosen format.
    let ext = files::extension_lower(&path);
    if Format::from_extension(&ext) != Some(format) {
        path.set_extension(format.extension());
    }
    Some(SaveResult { path, format, jpeg_quality: slider.doubleValue() / 100.0 })
}

// ---------------------------------------------------------------------------
// Rename
// ---------------------------------------------------------------------------

pub fn rename(mtm: MainThreadMarker, current: &Path) -> Option<PathBuf> {
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&ns("Rename File"));
    alert.setInformativeText(&ns("New file name (in the same folder):"));
    alert.addButtonWithTitle(&ns("Rename"));
    alert.addButtonWithTitle(&ns("Cancel"));
    let f = field(mtm, &files::file_name(current), rect(0.0, 0.0, 300.0, 24.0));
    alert.setAccessoryView(Some(&f));
    alert.window().setInitialFirstResponder(Some(&f));
    if alert.runModal() != NSAlertFirstButtonReturn {
        return None;
    }
    let name = f.stringValue().to_string();
    let name = name.trim();
    if name.is_empty() || name.contains('/') || name == files::file_name(current) {
        return None;
    }
    Some(current.with_file_name(name))
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// Build the properties text for an image file.
pub fn properties_text(path: &Path, img: &NSImage, zoom: Option<f64>) -> String {
    let mut out = String::new();
    let (w, h) = ops::pixel_size(img);
    out.push_str(&format!("File:        {}\n", files::file_name(path)));
    out.push_str(&format!("Folder:      {}\n", path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default()));
    if let Some(s) = files::file_size(path) {
        out.push_str(&format!("Size:        {}\n", files::human_size(s)));
    }
    out.push_str(&format!("Modified:    {}\n", files::modified_string(path)));
    out.push_str(&format!("Dimensions:  {w} × {h} px ({:.2} Mpx)\n", (w * h) as f64 / 1e6));
    let mp = w as f64 / h.max(1) as f64;
    out.push_str(&format!("Aspect:      {:.3}:1\n", mp));
    if let Some(z) = zoom {
        out.push_str(&format!("Zoom:        {:.0} %\n", z * 100.0));
    }

    // Bitmap details and EXIF come from a fresh rep read straight from the
    // file; the displayed NSImage may already have been modified.
    let url = NSURL::fileURLWithPath(&ns(&path.to_string_lossy()));
    if let Some(data) = NSData::dataWithContentsOfURL(&url) {
        if let Some(rep) = NSBitmapImageRep::imageRepWithData(&data) {
            let pts = rep.size();
            let pw = rep.pixelsWide() as f64;
            if pts.width > 0.0 {
                let dpi = pw / (pts.width / 72.0);
                out.push_str(&format!("Resolution:  {:.0} dpi ({:.1} px/cm)\n", dpi, dpi / 2.54));
                out.push_str(&format!("Print size:  {:.1} × {:.1} cm at that dpi\n", pts.width / 72.0 * 2.54, pts.height / 72.0 * 2.54));
            }
            out.push_str(&format!("Bit depth:   {} bits/sample, {} samples/pixel, alpha: {}\n", rep.bitsPerSample(), rep.samplesPerPixel(), if rep.hasAlpha() { "yes" } else { "no" }));
            out.push_str(&format!("Colour space: {}\n", rep.colorSpaceName()));
            let exif = rep.valueForProperty(unsafe { NSImageEXIFData });
            if let Some(exif) = exif {
                if let Some(dict) = exif.downcast_ref::<NSDictionary>() {
                    out.push_str("\nEXIF\n");
                    let (keys, values) = dict.to_vecs();
                    let mut pairs: Vec<(String, String)> = keys
                        .iter()
                        .zip(values.iter())
                        .map(|(k, v)| (describe(k), describe(v)))
                        .collect();
                    pairs.sort();
                    for (k, v) in pairs {
                        out.push_str(&format!("  {:<28} {}\n", k, v));
                    }
                }
            } else {
                out.push_str("\nNo EXIF data.\n");
            }
        }
    }
    out
}

fn describe(obj: &AnyObject) -> String {
    let d: Retained<NSString> = unsafe { msg_send![obj, description] };
    let s = d.to_string();
    // Flatten multi-line descriptions of arrays/dictionaries.
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn show_properties(mtm: MainThreadMarker, title: &str, text: &str) -> Retained<NSWindow> {
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect(0.0, 0.0, 620.0, 520.0),
            style,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setTitle(&ns(title));
    let scroll = NSTextView::scrollableTextView(mtm);
    scroll.setFrame(rect(0.0, 0.0, 620.0, 520.0));
    scroll.setAutoresizingMask(objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable);
    if let Some(doc) = scroll.documentView() {
        if let Some(tv) = doc.downcast_ref::<NSTextView>() {
            tv.setEditable(false);
            tv.setString(&ns(text));
            let font = NSFont::monospacedSystemFontOfSize_weight(12.0, unsafe { NSFontWeightRegular });
            tv.setFont(Some(&font));
        }
    }
    window.setContentView(Some(&scroll));
    window.center();
    // We keep our own Retained<NSWindow>; don't let AppKit free it on close.
    unsafe { window.setReleasedWhenClosed(false) };
    window.makeKeyAndOrderFront(None);
    window
}
