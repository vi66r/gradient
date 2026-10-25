//! A still picture of a page at the screen's resolution, for the shell to
//! draw in the page's place: WebKit draws above GPUI, so anything the shell
//! lays over a page (the address suggestions) needs the native view hidden,
//! and the picture keeps that from showing.
use super::Event;
use async_channel::Sender;
use block2::RcBlock;
use gpui::RenderImage;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSBitmapFormat, NSBitmapImageRep, NSColorRenderingIntent, NSColorSpace, NSImage,
};
use objc2_foundation::NSError;
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use std::sync::Arc;

pub fn take(view: &WKWebView, id: u64, events: Sender<Event>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // The whole view, at its own width; WebKit renders at the screen's scale.
    let config = unsafe { WKSnapshotConfiguration::new(mtm) };
    let done = RcBlock::new(move |image: *mut NSImage, _: *mut NSError| {
        let image = unsafe { image.as_ref() }.and_then(render_image);
        let _ = events.try_send(Event::Snapshot { id, image });
    });
    unsafe { view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &done) };
}

/// GPUI's BGRA pixels from an sRGB copy of the snapshot. Pages are opaque.
fn render_image(image: &NSImage) -> Option<Arc<RenderImage>> {
    let cg =
        unsafe { image.CGImageForProposedRect_context_hints(std::ptr::null_mut(), None, None) }?;
    let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
    let bitmap = bitmap.bitmapImageRepByConvertingToColorSpace_renderingIntent(
        &NSColorSpace::sRGBColorSpace(),
        NSColorRenderingIntent::Default,
    )?;
    let (wide, high) = (bitmap.pixelsWide(), bitmap.pixelsHigh());
    let bytes = bitmap.bitsPerPixel() / 8;
    if wide <= 0 || high <= 0 || bitmap.bitsPerSample() != 8 || bitmap.isPlanar() || bytes < 3 {
        return None;
    }
    let format = bitmap.bitmapFormat();
    if format.contains(NSBitmapFormat::FloatingPointSamples) {
        return None;
    }
    let alpha = bitmap.hasAlpha();
    let first = alpha && format.contains(NSBitmapFormat::AlphaFirst);
    let little = bytes == 4 && format.contains(NSBitmapFormat::ThirtyTwoBitLittleEndian);
    // Byte offsets of red, green and blue within a pixel.
    let [r, g, b] = match (first, little) {
        (false, false) => [0, 1, 2],
        (true, false) => [1, 2, 3],
        (false, true) => [3, 2, 1],
        (true, true) => [2, 1, 0],
    };
    let (wide, high, bytes) = (wide as usize, high as usize, bytes as usize);
    let stride = bitmap.bytesPerRow() as usize;
    let data = bitmap.bitmapData();
    if data.is_null() || stride < wide * bytes {
        return None;
    }
    // The bitmap owns `stride * high` bytes and outlives this read.
    let source = unsafe { std::slice::from_raw_parts(data, stride * high) };
    let mut pixels = Vec::with_capacity(wide * high * 4);
    for row in source.chunks_exact(stride) {
        for pixel in row[..wide * bytes].chunks_exact(bytes) {
            pixels.extend_from_slice(&[pixel[b], pixel[g], pixel[r], 0xff]);
        }
    }
    let buffer = image::RgbaImage::from_raw(wide as u32, high as u32, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}
