//! Image operations. Everything goes through one 8-bit RGBA buffer type so the
//! pixel work is plain Rust; AppKit is only used to decode, resample and
//! encode.

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapFormat, NSBitmapImageFileType, NSBitmapImageRep, NSCalibratedRGBColorSpace,
    NSCompositingOperation, NSGraphicsContext, NSImage, NSImageInterpolation, NSImageRep,
};
use objc2_foundation::{NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};

/// A straight (non-premultiplied) 8-bit RGBA buffer, row-major, top row first.
#[derive(Clone)]
pub struct Pixels {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Quality {
    Nearest,
    Low,
    Medium,
    High,
}

impl Quality {
    fn interpolation(self) -> NSImageInterpolation {
        match self {
            Quality::Nearest => NSImageInterpolation::None,
            Quality::Low => NSImageInterpolation::Low,
            Quality::Medium => NSImageInterpolation::Medium,
            Quality::High => NSImageInterpolation::High,
        }
    }
}

/// Pixel dimensions of an image (from its first representation; falls back
/// to the point size).
pub fn pixel_size(img: &NSImage) -> (usize, usize) {
    let reps = img.representations();
    if let Some(rep) = reps.iter().next() {
        let w = rep.pixelsWide();
        let h = rep.pixelsHigh();
        if w > 0 && h > 0 {
            return (w as usize, h as usize);
        }
    }
    let s = img.size();
    (s.width.max(1.0) as usize, s.height.max(1.0) as usize)
}

/// Make an empty 32-bit RGBA bitmap rep (premultiplied, as AppKit draws).
fn new_rep(w: usize, h: usize, premultiplied: bool) -> Retained<NSBitmapImageRep> {
    let format = if premultiplied {
        NSBitmapFormat::empty()
    } else {
        NSBitmapFormat::AlphaNonpremultiplied
    };
    unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bitmapFormat_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            w as isize,
            h as isize,
            8,
            4,
            true,
            false,
            NSCalibratedRGBColorSpace,
            format,
            (w * 4) as isize,
            32,
        )
        .expect("NSBitmapImageRep allocation failed")
    }
}

/// Draw `img` (optionally only `from` in image pixel coordinates, top-left
/// origin) into a fresh `w`×`h` bitmap with the given resampling quality.
pub fn rasterize(img: &NSImage, from: Option<(f64, f64, f64, f64)>, w: usize, h: usize, q: Quality) -> Pixels {
    let (iw, ih) = pixel_size(img);
    let rep = new_rep(w, h, true);
    let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)
        .expect("graphics context for bitmap");
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    ctx.setImageInterpolation(q.interpolation());
    // NSImage draws in points; we told it (on load) that points == pixels.
    let from_rect = match from {
        Some((x, y, cw, ch)) => {
            // Convert from top-left origin to NSImage's bottom-left origin.
            NSRect::new(NSPoint::new(x, ih as f64 - y - ch), NSSize::new(cw, ch))
        }
        None => NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(iw as f64, ih as f64)),
    };
    img.drawInRect_fromRect_operation_fraction(
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w as f64, h as f64)),
        from_rect,
        NSCompositingOperation::Copy,
        1.0,
    );
    ctx.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let mut px = pixels_from_rep(&rep);
    unpremultiply(&mut px);
    px
}

/// Copy a rep's bytes out. The rep must be 32-bit RGBA, non-planar
/// (which is what `new_rep` produces). Rows come out top-first.
fn pixels_from_rep(rep: &NSBitmapImageRep) -> Pixels {
    let w = rep.pixelsWide() as usize;
    let h = rep.pixelsHigh() as usize;
    let stride = rep.bytesPerRow() as usize;
    let src = rep.bitmapData();
    let mut data = vec![0u8; w * h * 4];
    unsafe {
        for y in 0..h {
            let row = src.add(y * stride);
            std::ptr::copy_nonoverlapping(row, data.as_mut_ptr().add(y * w * 4), w * 4);
        }
    }
    Pixels { w, h, data }
}

fn unpremultiply(px: &mut Pixels) {
    for p in px.data.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 0 || a == 255 {
            continue;
        }
        for c in &mut p[..3] {
            *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
}

/// Wrap a pixel buffer in an NSImage (via a non-premultiplied bitmap rep).
pub fn to_image(px: &Pixels) -> Retained<NSImage> {
    let rep = new_rep(px.w, px.h, false);
    let stride = rep.bytesPerRow() as usize;
    let dst = rep.bitmapData();
    unsafe {
        for y in 0..px.h {
            std::ptr::copy_nonoverlapping(
                px.data.as_ptr().add(y * px.w * 4),
                dst.add(y * stride),
                px.w * 4,
            );
        }
    }
    let img = NSImage::initWithSize(NSImage::alloc(), NSSize::new(px.w as f64, px.h as f64));
    let rep_ref: &NSImageRep = &rep;
    img.addRepresentation(rep_ref);
    img
}

/// Full-resolution pixels of an image, no resampling.
pub fn pixels_of(img: &NSImage) -> Pixels {
    let (w, h) = pixel_size(img);
    rasterize(img, None, w, h, Quality::Nearest)
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

pub fn rotate_cw(px: &Pixels) -> Pixels {
    let (w, h) = (px.w, px.h);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            // (x, y) -> (h-1-y, x) in the new (h wide, w high) image
            let nx = h - 1 - y;
            let ny = x;
            let s = (y * w + x) * 4;
            let d = (ny * h + nx) * 4;
            out[d..d + 4].copy_from_slice(&px.data[s..s + 4]);
        }
    }
    Pixels { w: h, h: w, data: out }
}

pub fn rotate_ccw(px: &Pixels) -> Pixels {
    let (w, h) = (px.w, px.h);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let nx = y;
            let ny = w - 1 - x;
            let s = (y * w + x) * 4;
            let d = (ny * h + nx) * 4;
            out[d..d + 4].copy_from_slice(&px.data[s..s + 4]);
        }
    }
    Pixels { w: h, h: w, data: out }
}

pub fn rotate_180(px: &Pixels) -> Pixels {
    let mut out = px.data.clone();
    let n = px.w * px.h;
    for i in 0..n / 2 {
        let j = n - 1 - i;
        let (a, b) = (i * 4, j * 4);
        for k in 0..4 {
            out.swap(a + k, b + k);
        }
    }
    Pixels { w: px.w, h: px.h, data: out }
}

pub fn flip_horizontal(px: &Pixels) -> Pixels {
    let mut out = px.data.clone();
    let row = px.w * 4;
    for y in 0..px.h {
        let r = &mut out[y * row..(y + 1) * row];
        for x in 0..px.w / 2 {
            let (a, b) = (x * 4, (px.w - 1 - x) * 4);
            for k in 0..4 {
                r.swap(a + k, b + k);
            }
        }
    }
    Pixels { w: px.w, h: px.h, data: out }
}

pub fn flip_vertical(px: &Pixels) -> Pixels {
    let row = px.w * 4;
    let mut out = Vec::with_capacity(px.data.len());
    for y in (0..px.h).rev() {
        out.extend_from_slice(&px.data[y * row..(y + 1) * row]);
    }
    Pixels { w: px.w, h: px.h, data: out }
}

/// Resample to a new size.
pub fn resize(img: &NSImage, w: usize, h: usize, q: Quality) -> Pixels {
    rasterize(img, None, w.max(1), h.max(1), q)
}

/// Crop to a rectangle given in image pixels, top-left origin.
pub fn crop(img: &NSImage, x: f64, y: f64, w: f64, h: f64) -> Pixels {
    let (iw, ih) = pixel_size(img);
    let x0 = x.max(0.0).floor();
    let y0 = y.max(0.0).floor();
    let x1 = (x + w).min(iw as f64).ceil();
    let y1 = (y + h).min(ih as f64).ceil();
    let cw = (x1 - x0).max(1.0);
    let ch = (y1 - y0).max(1.0);
    rasterize(img, Some((x0, y0, cw, ch)), cw as usize, ch as usize, Quality::Nearest)
}

/// Change the canvas size, keeping the image centred. `fill` is RGBA.
pub fn canvas(px: &Pixels, new_w: usize, new_h: usize, fill: [u8; 4]) -> Pixels {
    let mut out = Vec::with_capacity(new_w * new_h * 4);
    for _ in 0..new_w * new_h {
        out.extend_from_slice(&fill);
    }
    let ox = (new_w as isize - px.w as isize) / 2;
    let oy = (new_h as isize - px.h as isize) / 2;
    for y in 0..px.h {
        let ty = y as isize + oy;
        if ty < 0 || ty >= new_h as isize {
            continue;
        }
        for x in 0..px.w {
            let tx = x as isize + ox;
            if tx < 0 || tx >= new_w as isize {
                continue;
            }
            let s = (y * px.w + x) * 4;
            let d = (ty as usize * new_w + tx as usize) * 4;
            out[d..d + 4].copy_from_slice(&px.data[s..s + 4]);
        }
    }
    Pixels { w: new_w, h: new_h, data: out }
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

pub fn grayscale(px: &Pixels) -> Pixels {
    let mut out = px.data.clone();
    for p in out.chunks_exact_mut(4) {
        // Rec. 601 luma, integer math.
        let y = (p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114 + 500) / 1000;
        p[0] = y as u8;
        p[1] = y as u8;
        p[2] = y as u8;
    }
    Pixels { w: px.w, h: px.h, data: out }
}

pub fn negative(px: &Pixels) -> Pixels {
    let mut out = px.data.clone();
    for p in out.chunks_exact_mut(4) {
        p[0] = 255 - p[0];
        p[1] = 255 - p[1];
        p[2] = 255 - p[2];
    }
    Pixels { w: px.w, h: px.h, data: out }
}

/// Auto-adjust: stretch each channel so that 0.5 % of pixels clip at each
/// end. This is close to what IrfanView's "Auto adjust colors" does.
pub fn auto_adjust(px: &Pixels) -> Pixels {
    let n = (px.w * px.h) as u64;
    if n == 0 {
        return px.clone();
    }
    let clip = (n / 200).max(1); // 0.5 %
    let mut lut = [[0u8; 256]; 3];
    for c in 0..3 {
        let mut hist = [0u64; 256];
        for p in px.data.chunks_exact(4) {
            hist[p[c] as usize] += 1;
        }
        let mut lo = 0usize;
        let mut acc = 0u64;
        while lo < 255 && acc + hist[lo] < clip {
            acc += hist[lo];
            lo += 1;
        }
        let mut hi = 255usize;
        acc = 0;
        while hi > lo && acc + hist[hi] < clip {
            acc += hist[hi];
            hi -= 1;
        }
        let span = (hi as f64 - lo as f64).max(1.0);
        for (v, slot) in lut[c].iter_mut().enumerate() {
            let s = ((v as f64 - lo as f64) * 255.0 / span).round();
            *slot = s.clamp(0.0, 255.0) as u8;
        }
    }
    let mut out = px.data.clone();
    for p in out.chunks_exact_mut(4) {
        p[0] = lut[0][p[0] as usize];
        p[1] = lut[1][p[1] as usize];
        p[2] = lut[2][p[2] as usize];
    }
    Pixels { w: px.w, h: px.h, data: out }
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Jpeg,
    Png,
    Tiff,
    Gif,
    Bmp,
}

impl Format {
    pub fn from_extension(ext: &str) -> Option<Format> {
        match ext {
            "jpg" | "jpeg" | "jpe" | "jfif" => Some(Format::Jpeg),
            "png" => Some(Format::Png),
            "tif" | "tiff" => Some(Format::Tiff),
            "gif" => Some(Format::Gif),
            "bmp" => Some(Format::Bmp),
            _ => None,
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Tiff => "tif",
            Format::Gif => "gif",
            Format::Bmp => "bmp",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Format::Jpeg => "JPEG",
            Format::Png => "PNG",
            Format::Tiff => "TIFF",
            Format::Gif => "GIF",
            Format::Bmp => "BMP",
        }
    }
    pub const ALL: [Format; 5] = [Format::Jpeg, Format::Png, Format::Tiff, Format::Gif, Format::Bmp];
    fn file_type(self) -> NSBitmapImageFileType {
        match self {
            Format::Jpeg => NSBitmapImageFileType::JPEG,
            Format::Png => NSBitmapImageFileType::PNG,
            Format::Tiff => NSBitmapImageFileType::TIFF,
            Format::Gif => NSBitmapImageFileType::GIF,
            Format::Bmp => NSBitmapImageFileType::BMP,
        }
    }
}

/// Encode an image. `jpeg_quality` is 0.0–1.0. JPEG has no alpha, so the
/// image is composited onto white first.
pub fn encode(img: &NSImage, fmt: Format, jpeg_quality: f64) -> Option<Retained<NSData>> {
    let mut px = pixels_of(img);
    if fmt == Format::Jpeg || fmt == Format::Bmp {
        flatten_on_white(&mut px);
    }
    let rep = new_rep(px.w, px.h, false);
    let stride = rep.bytesPerRow() as usize;
    let dst = rep.bitmapData();
    unsafe {
        for y in 0..px.h {
            std::ptr::copy_nonoverlapping(
                px.data.as_ptr().add(y * px.w * 4),
                dst.add(y * stride),
                px.w * 4,
            );
        }
    }
    let props: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> = if fmt == Format::Jpeg {
        let key: &NSString = unsafe { objc2_app_kit::NSImageCompressionFactor };
        let val = objc2_foundation::NSNumber::new_f64(jpeg_quality.clamp(0.0, 1.0));
        let val_obj: &objc2::runtime::AnyObject = &val;
        NSDictionary::from_slices(&[key], &[val_obj])
    } else {
        NSDictionary::new()
    };
    unsafe { rep.representationUsingType_properties(fmt.file_type(), &props) }
}

fn flatten_on_white(px: &mut Pixels) {
    for p in px.data.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a == 255 {
            continue;
        }
        for c in &mut p[..3] {
            *c = ((*c as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
        p[3] = 255;
    }
}

/// Load an image from disk and normalise its point size to its pixel size,
/// so that 1 point == 1 pixel everywhere else in the app.
pub fn load(path: &std::path::Path) -> Option<Retained<NSImage>> {
    let url = objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let img = NSImage::initWithContentsOfURL(NSImage::alloc(), &url)?;
    let (w, h) = pixel_size(&img);
    img.setSize(NSSize::new(w as f64, h as f64));
    Some(img)
}
