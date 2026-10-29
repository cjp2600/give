//! Filesystem notifications include ignored paths and coalesce bursts into a refresh.
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct ProjectWatcher {
    _watcher: RecommendedWatcher,
    dirty: Arc<AtomicBool>,
}

impl ProjectWatcher {
    pub fn new(root: &Path) -> notify::Result<Self> {
        let dirty = Arc::new(AtomicBool::new(false));
        let pending = dirty.clone();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                // Reading a preview or scanning the catalog must not trigger another scan.
                if event.as_ref().map_or(true, |event| !event.kind.is_access()) {
                    pending.store(true, Ordering::Release);
                }
            })?;
        watcher.watch(root, RecursiveMode::Recursive)?;
        Ok(Self {
            _watcher: watcher,
            dirty,
        })
    }

    /// Consume only when the UI can start a refresh, retaining events received mid-scan.
    pub fn take_changed(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }
}
