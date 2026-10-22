//! Selection-reader support for macOS; it doesn't intercept the reader's hotkey.
use gpui::{Bounds, Pixels, Window};
#[cfg(target_os = "macos")]
use std::ffi::CString;

#[derive(Default)]
pub struct Text {
    value: String,
    selected: String,
    sent_value: bool,
    sent_selection: bool,
    /// Focus, visibility and frame last given to AppKit; unchanged frames skip the native call.
    sent_state: Option<(bool, bool, Bounds<Pixels>, std::ops::Range<usize>)>,
    pub range: std::ops::Range<usize>,
}
impl Text {
    pub fn invalidate(&mut self) {
        self.sent_value = false;
        self.sent_selection = false;
        self.sent_state = None;
    }
    pub fn set_value(&mut self, text: String) {
        if self.value != text {
            self.value = text;
            self.sent_value = false;
        }
    }
    pub fn set_selection(&mut self, text: String, range: std::ops::Range<usize>) {
        if self.selected != text {
            self.selected = text;
            self.sent_selection = false;
        }
        self.range = range;
    }
    pub fn publish(
        &mut self,
        index: u32,
        label: &str,
        focused: bool,
        visible: bool,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) {
        #[cfg(not(target_os = "macos"))]
        let _ = (index, label, window);
        let state = (focused, visible, bounds, self.range.clone());
        if self.sent_value && self.sent_selection && self.sent_state.as_ref() == Some(&state) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            // GPUI's test windows have no native AppKit view.
            if cfg!(test) {
                return;
            }
            let Ok(handle) = HasWindowHandle::window_handle(window) else {
                return;
            };
            let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                return;
            };
            let label = CString::new(label).unwrap_or_default();
            let value = (!self.sent_value)
                .then(|| CString::new(self.value.replace('\0', "�")).unwrap_or_default());
            let selection = (!self.sent_selection)
                .then(|| CString::new(self.selected.replace('\0', "�")).unwrap_or_default());
            // GPUI invokes Render on the AppKit main thread and keeps this NSView
            // alive for the duration of the call. Native code copies the strings.
            unsafe {
                give_selection_update(
                    handle.ns_view.as_ptr(),
                    index,
                    label.as_ptr(),
                    value.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                    selection.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
                    self.range.start,
                    self.range.end.saturating_sub(self.range.start),
                    focused,
                    visible,
                    f32::from(bounds.origin.x) as f64,
                    f32::from(bounds.origin.y) as f64,
                    f32::from(bounds.size.width) as f64,
                    f32::from(bounds.size.height) as f64,
                );
            }
        }
        self.sent_value = true;
        self.sent_selection = true;
        self.sent_state = Some(state);
    }
}
#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn give_selection_update(
        view: *mut std::ffi::c_void,
        index: u32,
        label: *const std::ffi::c_char,
        text: *const std::ffi::c_char,
        selection: *const std::ffi::c_char,
        start: usize,
        length: usize,
        focused: bool,
        visible: bool,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    );
}
