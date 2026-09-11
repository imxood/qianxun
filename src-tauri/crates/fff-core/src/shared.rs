use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use crate::error::Error;
use crate::file_picker::FilePicker;
use crate::rescan_throttle::RescanThrottle;
use crate::watch::{WatchEvent, WatchId, WatchOptions, WatchRegistry};
use log::error;

/// Poll `done` every 10ms until it returns `true`, or until `timeout` elapses.
fn poll_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while !done() {
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    true
}

/// Thread-safe shared handle to the [`FilePicker`] instance.
/// This accumulates only asynchronous non-blocking operations against the
/// file picker: creating, triggering rescans and so on.
///
/// For blocking access use internal picker via `.read()` or `.write()`
#[derive(Clone, Default)]
pub struct SharedFilePicker(pub(crate) Arc<SharedPickerInner>);

#[allow(dead_code)]
pub struct SharedPickerInner {
    picker: parking_lot::RwLock<Option<FilePicker>>,
    watchers: Arc<WatchRegistry>,
    rescan_throttle: RescanThrottle,
}

impl Default for SharedPickerInner {
    fn default() -> Self {
        Self {
            picker: parking_lot::RwLock::new(None),
            watchers: Arc::new(WatchRegistry::default()),
            rescan_throttle: RescanThrottle::default(),
        }
    }
}

/// Non-owning handle to a [`SharedPicker`].
#[derive(Clone)]
pub(crate) struct WeakFilePicker(Weak<SharedPickerInner>);

impl WeakFilePicker {
    /// Try to promote the weak handle back to a strong [`SharedPicker`].
    pub(crate) fn upgrade(&self) -> Option<SharedFilePicker> {
        self.0.upgrade().map(SharedFilePicker)
    }
}

impl std::fmt::Debug for SharedFilePicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedPicker").field(&"..").finish()
    }
}

impl SharedFilePicker {
    pub fn read(&self) -> Result<parking_lot::RwLockReadGuard<'_, Option<FilePicker>>, Error> {
        Ok(self.0.picker.read())
    }

    pub fn write(&self) -> Result<parking_lot::RwLockWriteGuard<'_, Option<FilePicker>>, Error> {
        Ok(self.0.picker.write())
    }

    /// Signal the background scan to cancel. Non-blocking.
    pub fn cancel(&self) {
        if let Ok(guard) = self.read() {
            if let Some(picker) = guard.as_ref() {
                picker.cancel();
            }
        }
    }

    /// Produce a non-owning handle to the same inner picker.
    pub(crate) fn weaken(&self) -> WeakFilePicker {
        WeakFilePicker(Arc::downgrade(&self.0))
    }

    /// Block until the background filesystem scan finishes.
    pub fn wait_for_scan(&self, timeout: Duration) -> bool {
        let signal = {
            let guard = self.0.picker.read();
            match &*guard {
                Some(picker) => Arc::clone(&picker.signals.scanning),
                None => return true,
            }
        };
        poll_until(timeout, || {
            !signal.load(std::sync::atomic::Ordering::Acquire)
        })
    }

    /// Block until the background file watcher is ready.
    pub fn wait_for_watcher(&self, timeout: Duration) -> bool {
        let watch_ready_signal = {
            let guard = self.0.picker.read();
            match &*guard {
                Some(picker) => Arc::clone(&picker.signals.watcher_ready),
                None => return true,
            }
        };
        poll_until(timeout, || {
            watch_ready_signal.load(std::sync::atomic::Ordering::Acquire)
        })
    }

    /// Returns the live file count of the scanned index.
    pub fn live_file_count(&self) -> usize {
        let guard = self.0.picker.read();
        if let Some(picker) = guard.as_ref() {
            picker.live_file_count()
        } else {
            0
        }
    }

    /// Subscribe to filesystem changes matching `pattern`.
    pub fn watch(
        &self,
        pattern: &str,
        options: WatchOptions,
        callback: impl Fn(WatchId, &[WatchEvent]) + Send + Sync + 'static,
    ) -> Result<WatchId, Error> {
        let (base_path, has_watcher, watcher_ready) = {
            let guard = self.read()?;
            let picker = guard.as_ref().ok_or(Error::FilePickerMissing)?;
            (
                picker.base_path().to_path_buf(),
                picker.has_watcher(),
                picker.is_watcher_ready(),
            )
        };

        if !has_watcher {
            return Err(Error::WatcherDisabled);
        }
        if !watcher_ready {
            return Err(Error::WatcherNotReady);
        }

        self.0
            .watchers
            .subscribe(&base_path, pattern, options, Box::new(callback))
    }

    /// Remove a watch subscription. Returns `true` if the id was active.
    pub fn unwatch(&self, id: WatchId) -> bool {
        self.0.watchers.unsubscribe(id)
    }

    /// Return whether a watch subscription is active.
    pub fn is_watch_active(&self, id: WatchId) -> bool {
        self.0.watchers.contains(id)
    }

    /// Remove every subscription without waiting.
    pub fn shutdown_watches(&self) {
        self.0.watchers.shutdown();
    }

    /// Remove every subscription and wait for an executing callback.
    pub fn shutdown_watches_and_wait(&self) {
        self.0.watchers.shutdown_and_wait();
    }

    pub(crate) fn rebase_watches(&self, base_path: &Path) {
        self.0.watchers.rebase(base_path);
    }

    pub(crate) fn watch_registry(&self) -> &Arc<WatchRegistry> {
        &self.0.watchers
    }

    /// Signal the background scan to do a full rescan for the given reason.
    ///
    /// This is non-blocking; the picker's internal rescan throttle decides
    /// whether to actually start a new scan.
    pub fn trigger_full_rescan_with_reason(
        &self,
        _reason: crate::file_picker::RescanReason,
    ) -> bool {
        if let Ok(mut guard) = self.write() {
            if let Some(ref mut picker) = *guard {
                if let Err(e) = picker.rescan_sync() {
                    error!("Full rescan failed: {}", e);
                }
                return true;
            }
        }
        false
    }
}
