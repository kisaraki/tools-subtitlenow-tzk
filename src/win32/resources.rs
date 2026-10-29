//! Small owning wrappers. Every object is created and released on the discovery thread.

use std::{marker::PhantomData, rc::Rc};

use windows::{
    Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
    core::PWSTR,
};

use crate::audio::sessions::DiscoveryError;

pub(super) fn native(operation: &'static str, error: windows::core::Error) -> DiscoveryError {
    DiscoveryError::Native {
        operation,
        code: error.code().0,
    }
}

pub(super) struct Apartment(PhantomData<Rc<()>>);

impl Apartment {
    pub fn new() -> Result<Self, DiscoveryError> {
        // SAFETY: called only on the dedicated worker. Both S_OK and S_FALSE require one
        // matching CoUninitialize; a failed initialization creates no guard. !Send/!Sync.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| native("CoInitializeEx", error))?;
        Ok(Self(PhantomData))
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: the guard stays on its initialization thread, and outlives all COM fields.
        unsafe { CoUninitialize() };
    }
}

struct TaskString(PWSTR);

impl Drop for TaskString {
    fn drop(&mut self) {
        // SAFETY: ownership comes exclusively from documented CoTaskMem-allocated outputs.
        unsafe { CoTaskMemFree(Some(self.0.as_ptr().cast())) };
    }
}

/// Caller transfers a COM task-allocated, NUL-terminated UTF-16 string (or null).
pub(super) unsafe fn take_string(pointer: PWSTR) -> Result<String, DiscoveryError> {
    const MAX_NATIVE_STRING_UNITS: usize = 32_768;
    let owned = TaskString(pointer);
    if owned.0.is_null() {
        return Ok(String::new());
    }
    let mut units = Vec::new();
    for index in 0..MAX_NATIVE_STRING_UNITS {
        // SAFETY: caller guarantees a valid terminated allocation; stop at the first NUL.
        let unit = unsafe { *owned.0.as_ptr().add(index) };
        if unit == 0 {
            return String::from_utf16(&units)
                .map_err(|_| DiscoveryError::InvalidData("invalid_utf16"));
        }
        units.push(unit);
    }
    Err(DiscoveryError::InvalidData("native_string_too_long"))
}

pub(super) fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}
