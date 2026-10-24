//! Downloads through `WKDownload` directly. Wry 0.57 keeps the download object
//! to itself, so it can neither report progress nor cancel; `Protection` hands
//! each new download here instead of to Wry's delegate.
use crate::engine::Event;
use async_channel::Sender;
use block2::DynBlock;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    rc::Retained,
    runtime::{NSObject, NSObjectProtocol, ProtocolObject},
};
use objc2_foundation::{NSData, NSError, NSProgressReporting, NSString, NSURL, NSURLResponse};
use objc2_web_kit::{WKDownload, WKDownloadDelegate};
use std::{cell::RefCell, collections::HashMap, path::PathBuf};

struct Entry {
    download: Retained<WKDownload>,
    // `WKDownload.delegate` is weak; the entry keeps it alive.
    _delegate: Retained<Delegate>,
    /// Set once the destination is chosen.
    path: Option<PathBuf>,
}

thread_local! {
    // Downloads in flight, by ID. WebKit delivers every callback on the UI thread.
    static ACTIVE: RefCell<HashMap<u64, Entry>> = RefCell::new(HashMap::new());
    static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

pub struct Ivars {
    id: u64,
    tab: u64,
    events: Sender<Event>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    pub struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl WKDownloadDelegate for Delegate {
        #[unsafe(method(download:decideDestinationUsingResponse:suggestedFilename:completionHandler:))]
        unsafe fn destination(
            &self,
            _download: &WKDownload,
            _response: &NSURLResponse,
            suggested: &NSString,
            handler: &DynBlock<dyn Fn(*mut NSURL)>,
        ) {
            let (id, tab, events) = (
                self.ivars().id,
                self.ivars().tab,
                self.ivars().events.clone(),
            );
            let dir = crate::downloads::dir();
            let name = crate::downloads::file_name(&suggested.to_string());
            // Two downloads of one name may both be in flight before either
            // file exists; reserve chosen paths until they finish.
            let path = ACTIVE.with_borrow(|active| {
                crate::downloads::unique(&dir, &name, |path| {
                    crate::downloads::exists(path)
                        || active
                            .values()
                            .any(|entry| entry.path.as_deref() == Some(path))
                })
            });
            if let Err(error) = std::fs::create_dir_all(&dir) {
                log::warn!("Downloads folder: {error}");
                let _entry = take(id);
                handler.call((std::ptr::null_mut(),));
                let _ = events.try_send(Event::Notice(
                    "The download couldn’t start: the Downloads folder isn’t available.".into(),
                ));
                return;
            }
            ACTIVE.with_borrow_mut(|active| {
                if let Some(entry) = active.get_mut(&id) {
                    entry.path = Some(path.clone());
                }
            });
            let url = NSURL::fileURLWithPath_isDirectory(
                &NSString::from_str(&path.to_string_lossy()),
                false,
            );
            handler.call((Retained::as_ptr(&url).cast_mut(),));
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(name);
            let _ = events.try_send(Event::DownloadStarted {
                id,
                tab,
                name,
                path,
            });
        }

        #[unsafe(method(downloadDidFinish:))]
        unsafe fn finished(&self, _download: &WKDownload) {
            let (id, events) = (self.ivars().id, self.ivars().events.clone());
            // The entry holds the only strong reference to this delegate.
            let _entry = take(id);
            let _ = events.try_send(Event::DownloadEnded { id, ok: true });
        }

        #[unsafe(method(download:didFailWithError:resumeData:))]
        unsafe fn failed(&self, _download: &WKDownload, error: &NSError, _resume: Option<&NSData>) {
            let (id, events) = (self.ivars().id, self.ivars().events.clone());
            log::info!("Download {id} failed: {}", error.localizedDescription());
            let entry = take(id);
            // WebKit leaves the partial file behind.
            if let Some(path) = entry.as_ref().and_then(|entry| entry.path.as_ref()) {
                let _ = std::fs::remove_file(path);
            }
            let _ = events.try_send(Event::DownloadEnded { id, ok: false });
        }
    }
);

/// Removes a download from the registry. Callers inside its delegate keep the
/// entry alive until they return.
fn take(id: u64) -> Option<Entry> {
    ACTIVE.with_borrow_mut(|active| active.remove(&id))
}

/// Takes over a download WebKit just started for tab `tab`.
pub fn adopt(download: &WKDownload, tab: u64, events: Sender<Event>) {
    let mtm = MainThreadMarker::new().expect("WebKit downloads start on the UI thread");
    let id = NEXT.replace(NEXT.get() + 1);
    let object = Delegate::alloc(mtm).set_ivars(Ivars { id, tab, events });
    let delegate: Retained<Delegate> = unsafe { msg_send![super(object), init] };
    unsafe { download.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
    ACTIVE.with_borrow_mut(|active| {
        active.insert(
            id,
            Entry {
                download: download.retain(),
                _delegate: delegate,
                path: None,
            },
        )
    });
}

/// Bytes received so far, and the expected total when the server sent one.
pub fn progress(id: u64) -> Option<(i64, Option<i64>)> {
    ACTIVE.with_borrow(|active| {
        let progress = active.get(&id)?.download.progress();
        let total = progress.totalUnitCount();
        Some((progress.completedUnitCount(), (total > 0).then_some(total)))
    })
}

/// Stops a download and removes its partial file. Its delegate is released
/// first, so no further event arrives for it.
pub fn cancel(id: u64) {
    let Some(entry) = take(id) else {
        return;
    };
    unsafe { entry.download.cancel(None) };
    if let Some(path) = entry.path {
        let _ = std::fs::remove_file(path);
    }
}

/// Stops every download, for quitting.
pub fn cancel_all() {
    let ids: Vec<u64> = ACTIVE.with_borrow(|active| active.keys().copied().collect());
    for id in ids {
        cancel(id);
    }
}

/// Selects the file in a Finder window.
pub fn reveal(path: &std::path::Path) {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::NSArray;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSWorkspace::sharedWorkspace()
        .activateFileViewerSelectingURLs(&NSArray::from_retained_slice(&[url]));
}
