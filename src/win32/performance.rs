//! Read-only queries of our own process or our exact owned capture helper.
use crate::performance::resources::{ProcessProbe, ProcessSample, ResourceError};
use std::{
    mem::size_of,
    os::windows::io::{AsRawHandle, OwnedHandle},
    sync::Arc,
    time::Instant,
};
use windows::Win32::{
    Foundation::{FILETIME, HANDLE},
    System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX},
        Threading::{GetCurrentProcess, GetProcessHandleCount, GetProcessTimes},
    },
};
pub struct NativeProcessProbe {
    owned: Option<OwnedHandle>,
}
impl NativeProcessProbe {
    pub fn current() -> Arc<dyn ProcessProbe> {
        Arc::new(Self { owned: None })
    }
    pub(crate) fn owned(handle: OwnedHandle) -> Arc<dyn ProcessProbe> {
        Arc::new(Self {
            owned: Some(handle),
        })
    }
}
fn filetime(time: FILETIME) -> u64 {
    (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
}
impl ProcessProbe for NativeProcessProbe {
    fn sample(&self) -> Result<ProcessSample, ResourceError> {
        let mut created = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let mut memory = PROCESS_MEMORY_COUNTERS_EX {
            cb: size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let mut handles = 0;
        // SAFETY: pseudo handle is never closed; an owned handle stays live through
        // this call and has query/read rights. Correct EX size/prefix and writable
        // FILETIME/count pointers; no COM or cross-thread pointers escape.
        unsafe {
            let handle = self
                .owned
                .as_ref()
                .map_or_else(|| GetCurrentProcess(), |h| HANDLE(h.as_raw_handle()));
            GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user)
                .map_err(|e| ResourceError::Native(e.code().0))?;
            let at = Instant::now();
            GetProcessMemoryInfo(
                handle,
                (&mut memory as *mut PROCESS_MEMORY_COUNTERS_EX).cast(),
                memory.cb,
            )
            .map_err(|e| ResourceError::Native(e.code().0))?;
            GetProcessHandleCount(handle, &mut handles)
                .map_err(|e| ResourceError::Native(e.code().0))?;
            Ok(ProcessSample {
                at,
                created_100ns: filetime(created),
                cpu_100ns: filetime(kernel)
                    .checked_add(filetime(user))
                    .ok_or(ResourceError::InvalidSample)?,
                private_commit_bytes: memory.PrivateUsage as u64,
                working_set_bytes: memory.WorkingSetSize as u64,
                peak_working_set_bytes: memory.PeakWorkingSetSize as u64,
                handles,
            })
        }
    }
}
