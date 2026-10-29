//! Core Audio discovery on one MTA worker. Creation callbacks retain agile references;
//! polling reconciles devices and reads state/meters without acquiring audio samples.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
};

use windows::{
    Win32::{
        Devices::FunctionDiscovery::PKEY_Device_FriendlyName,
        Foundation::S_OK,
        Media::Audio::{
            AudioSessionStateActive, AudioSessionStateExpired, AudioSessionStateInactive,
            DEVICE_STATE_ACTIVE, Endpoints::IAudioMeterInformation, IAudioSessionControl,
            IAudioSessionControl2, IAudioSessionManager2, IAudioSessionNotification,
            IAudioSessionNotification_Impl, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
            MMDeviceEnumerator, eRender,
        },
        System::Com::{
            CLSCTX_ALL, CoCreateInstance, STGM_READ,
            StructuredStorage::{PROPVARIANT, PropVariantClear, PropVariantToStringAlloc},
        },
    },
    core::{AgileReference, HRESULT, Interface, Ref, implement},
};

use crate::audio::{
    discovery::AudioSessionScanner,
    sessions::{
        ApplicationIcon, AudioSource, AudioSourceId, DiscoveryError, MAX_DEVICES, MAX_SESSIONS,
        MAX_TITLE_CHARS, ProcessScope, ScanReport, SessionState, normalized_level,
    },
};

use super::{
    icon, process,
    resources::{Apartment, native, nonempty, take_string},
};

/// Defined by audiopolicy.h as AUDCLNT_SUCCESS(0x00d); not exposed by windows 0.62.
const AUDCLNT_S_NO_SINGLE_PROCESS: HRESULT = HRESULT(0x0889_000D);

struct NewSession {
    endpoint: String,
    registration: u64,
    control: AgileReference<IAudioSessionControl>,
}

#[implement(IAudioSessionNotification)]
struct SessionNotification {
    endpoint: String,
    registration: u64,
    sender: SyncSender<NewSession>,
    lost: Arc<AtomicBool>,
}

#[allow(non_snake_case)]
impl IAudioSessionNotification_Impl for SessionNotification_Impl {
    fn OnSessionCreated(
        &self,
        newsession: Ref<'_, IAudioSessionControl>,
    ) -> windows::core::Result<()> {
        let Some(control) = newsession.as_ref() else {
            return Ok(());
        };
        // AgileReference provides documented marshaling, not an unsafe Send assertion.
        match AgileReference::new(control) {
            Ok(control) => {
                if self
                    .sender
                    .try_send(NewSession {
                        endpoint: self.endpoint.clone(),
                        registration: self.registration,
                        control,
                    })
                    .is_err()
                {
                    self.lost.store(true, Ordering::Release);
                }
            }
            Err(_) => self.lost.store(true, Ordering::Release),
        }
        Ok(())
    }
}

struct TrackedSession {
    process_created: Option<u64>,
    control: IAudioSessionControl2,
    pid: u32,
    executable: Option<String>,
    icon_key: Option<String>,
    icon: Option<Arc<ApplicationIcon>>,
}

struct TrackedDevice {
    manager: IAudioSessionManager2,
    notification: IAudioSessionNotification,
    registration: u64,
    name: Option<String>,
    sessions: BTreeMap<String, TrackedSession>,
}

impl Drop for TrackedDevice {
    fn drop(&mut self) {
        // SAFETY: both interfaces are owned and live on the registering MTA thread. The
        // callback owns only a bounded sender, so there is no manager/callback ownership cycle.
        if let Err(error) = unsafe {
            self.manager
                .UnregisterSessionNotification(&self.notification)
        } {
            tracing::warn!(hresult = error.code().0, "音訊探索通知取消註冊失敗");
        }
    }
}

struct PropertyValue(PROPVARIANT);
impl Drop for PropertyValue {
    fn drop(&mut self) {
        // SAFETY: GetValue transferred ownership of this initialized PROPVARIANT.
        let _ = unsafe { PropVariantClear(&mut self.0) };
    }
}

fn device_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: owned interface on initialized MTA; property is read only. Each allocated
    // PROPVARIANT and converted string has exactly one owner and is released on every path.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = PropertyValue(store.GetValue(&PKEY_Device_FriendlyName).ok()?);
        nonempty(take_string(PropVariantToStringAlloc(&value.0).ok()?).ok()?)
    }
}

impl TrackedDevice {
    fn new(
        device: &IMMDevice,
        id: &str,
        registration: u64,
        sender: SyncSender<NewSession>,
        lost: Arc<AtomicBool>,
    ) -> Result<Self, DiscoveryError> {
        // SAFETY: initialized MTA and a live render endpoint. No capture/audio client is opened.
        let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
            .map_err(|error| native("ActivateSessionManager", error))?;
        let notification: IAudioSessionNotification = SessionNotification {
            endpoint: id.to_owned(),
            registration,
            sender,
            lost,
        }
        .into();
        // SAFETY: callback is a COM implementation retained by this device and by registration.
        unsafe { manager.RegisterSessionNotification(&notification) }
            .map_err(|error| native("RegisterSessionNotification", error))?;
        Ok(Self {
            manager,
            notification,
            registration,
            name: device_name(device),
            sessions: BTreeMap::new(),
        })
    }

    fn insert(&mut self, control: IAudioSessionControl, capacity: usize, report: &mut ScanReport) {
        let result = (|| {
            let control: IAudioSessionControl2 = control
                .cast()
                .map_err(|error| native("SessionControl2", error))?;
            // SAFETY: owned session interface in MTA. Enumerators can retain expired entries;
            // do not reinsert them after pruning, otherwise they could consume capacity forever.
            if unsafe { control.GetState() }.map_err(|error| native("GetState", error))?
                == AudioSessionStateExpired
            {
                return Ok(());
            }
            // SAFETY: live session interface in MTA; returned string uses CoTaskMem ownership.
            let instance = unsafe {
                take_string(
                    control
                        .GetSessionInstanceIdentifier()
                        .map_err(|error| native("GetSessionInstanceIdentifier", error))?,
                )
            }?;
            if instance.is_empty() {
                return Err(DiscoveryError::InvalidData("empty_session_id"));
            }
            if self.sessions.contains_key(&instance) {
                return Ok(());
            }
            if self.sessions.len() >= capacity {
                report.limit_reached = true;
                return Ok(());
            }
            self.sessions.insert(
                instance,
                TrackedSession {
                    control,
                    pid: 0,
                    process_created: None,
                    executable: None,
                    icon_key: None,
                    icon: None,
                },
            );
            Ok(())
        })();
        if result.is_err() {
            report.skipped_sessions += 1;
        }
    }

    fn reconcile(
        &mut self,
        capacity: usize,
        report: &mut ScanReport,
        cancelled: &AtomicBool,
    ) -> Result<(), DiscoveryError> {
        // Release expired/invalid sessions before accepting new ones so a busy process cannot
        // permanently occupy the bounded collection with dead entries.
        self.sessions.retain(|_, session| {
            // SAFETY: each interface was resolved/created in the same MTA.
            match unsafe { session.control.GetState() } {
                Ok(state) => state != AudioSessionStateExpired,
                Err(_) => {
                    report.skipped_sessions += 1;
                    false
                }
            }
        });
        // SAFETY: manager is owned in MTA. GetCount after registration starts notifications;
        // enumeration supplements the callback-maintained list, not replaces it.
        let enumerator = unsafe { self.manager.GetSessionEnumerator() }
            .map_err(|error| native("GetSessionEnumerator", error))?;
        // SAFETY: live enumerator, generated binding supplies the writable out value.
        let count =
            unsafe { enumerator.GetCount() }.map_err(|error| native("GetSessionCount", error))?;
        if count < 0 {
            return Err(DiscoveryError::InvalidData("negative_session_count"));
        }
        if count as usize > MAX_SESSIONS {
            report.limit_reached = true;
        }
        for index in 0..count.min(MAX_SESSIONS as i32) {
            if cancelled.load(Ordering::Acquire) {
                return Err(DiscoveryError::Cancelled);
            }
            // SAFETY: index is within GetCount; races with session teardown return an error.
            match unsafe { enumerator.GetSession(index) } {
                Ok(control) => self.insert(control, capacity, report),
                Err(_) => report.skipped_sessions += 1,
            }
        }
        Ok(())
    }
}

/// !Send/!Sync through Apartment. Construct only inside DiscoveryMonitor's factory.
pub struct WindowsSessionScanner {
    devices: BTreeMap<String, TrackedDevice>,
    receiver: Receiver<NewSession>,
    sender: SyncSender<NewSession>,
    lost: Arc<AtomicBool>,
    next_registration: u64,
    // Rust drops fields in declaration order: all native interfaces precede COM teardown.
    _apartment: Apartment,
}

impl WindowsSessionScanner {
    pub fn new() -> Result<Self, DiscoveryError> {
        let apartment = Apartment::new()?;
        let (sender, receiver) = mpsc::sync_channel(MAX_SESSIONS);
        Ok(Self {
            devices: BTreeMap::new(),
            receiver,
            sender,
            lost: Arc::new(AtomicBool::new(false)),
            next_registration: 0,
            _apartment: apartment,
        })
    }

    fn scan_native(&mut self, cancelled: &AtomicBool) -> Result<ScanReport, DiscoveryError> {
        let mut report = ScanReport::default();
        // SAFETY: worker apartment is initialized. Recreating the enumerator allows recovery
        // after endpoint/service changes without carrying stale enumerator state.
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(|error| native("CreateDeviceEnumerator", error))?;
        // SAFETY: render endpoints only; no microphone/system-mix capture is initialized.
        let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) }
            .map_err(|error| native("EnumAudioEndpoints", error))?;
        // SAFETY: live collection, generated binding owns its out parameter.
        let count =
            unsafe { collection.GetCount() }.map_err(|error| native("GetDeviceCount", error))?;
        report.device_count = count as usize;
        report.limit_reached = count as usize > MAX_DEVICES;
        let mut active = BTreeSet::new();
        for index in 0..count.min(MAX_DEVICES as u32) {
            if cancelled.load(Ordering::Acquire) {
                return Err(DiscoveryError::Cancelled);
            }
            let endpoint = (|| {
                // SAFETY: index within count, live collection. GetId transfers string ownership.
                let device = unsafe { collection.Item(index) }
                    .map_err(|error| native("GetDevice", error))?;
                let id = unsafe {
                    take_string(
                        device
                            .GetId()
                            .map_err(|error| native("GetDeviceId", error))?,
                    )
                }?;
                if id.is_empty() {
                    return Err(DiscoveryError::InvalidData("empty_device_id"));
                }
                active.insert(id.clone());
                if !self.devices.contains_key(&id) {
                    self.next_registration = self
                        .next_registration
                        .checked_add(1)
                        .ok_or(DiscoveryError::InvalidData("registration_exhausted"))?;
                    let device = TrackedDevice::new(
                        &device,
                        &id,
                        self.next_registration,
                        self.sender.clone(),
                        Arc::clone(&self.lost),
                    )?;
                    self.devices.insert(id, device);
                }
                Ok(())
            })();
            if endpoint.is_err() {
                report.unavailable_devices += 1;
            }
        }
        self.devices.retain(|id, _| active.contains(id));
        let mut total: usize = self
            .devices
            .values()
            .map(|device| device.sessions.len())
            .sum();
        self.devices.retain(|_, device| {
            let previous = device.sessions.len();
            let capacity = MAX_SESSIONS.saturating_sub(total - previous);
            let ok = device.reconcile(capacity, &mut report, cancelled).is_ok();
            total = total - previous + if ok { device.sessions.len() } else { 0 };
            if !ok {
                report.unavailable_devices += 1;
            }
            ok
        });
        // Drain at most one mailbox's capacity per tick; producers cannot starve polling.
        for event in self.receiver.try_iter().take(MAX_SESSIONS) {
            let Some(device) = self.devices.get_mut(&event.endpoint) else {
                continue;
            };
            if device.registration != event.registration {
                continue;
            }
            let before = device.sessions.len();
            match event.control.resolve() {
                Ok(control) => device.insert(
                    control,
                    MAX_SESSIONS.saturating_sub(total - before),
                    &mut report,
                ),
                Err(_) => report.skipped_sessions += 1,
            }
            total = total - before + device.sessions.len();
        }
        report.notifications_lost = self.lost.swap(false, Ordering::AcqRel);
        let windows = process::window_titles();
        report.windows_unavailable = windows.unavailable;
        report.limit_reached |= windows.limit_reached;
        for (endpoint, device) in &mut self.devices {
            for (instance, session) in &mut device.sessions {
                if cancelled.load(Ordering::Acquire) {
                    return Err(DiscoveryError::Cancelled);
                }
                match session.snapshot(endpoint, instance, &device.name, &windows.titles) {
                    Ok(Some(source)) => report.sources.push(source),
                    Ok(None) => {}
                    Err(_) => report.skipped_sessions += 1,
                }
            }
        }
        report.sources.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(report)
    }
}

impl AudioSessionScanner for WindowsSessionScanner {
    fn scan(&mut self, cancelled: &AtomicBool) -> Result<ScanReport, DiscoveryError> {
        let result = self.scan_native(cancelled);
        if result.is_err() {
            self.devices.clear();
        }
        result
    }
}

impl TrackedSession {
    fn snapshot(
        &mut self,
        endpoint: &str,
        instance: &str,
        endpoint_name: &Option<String>,
        titles: &BTreeMap<u32, String>,
    ) -> Result<Option<AudioSource>, DiscoveryError> {
        // SAFETY: all calls below operate on an owned session interface in its initialized MTA.
        let state =
            unsafe { self.control.GetState() }.map_err(|error| native("GetState", error))?;
        if state == AudioSessionStateExpired {
            return Ok(None);
        }
        let state = if state == AudioSessionStateActive {
            SessionState::Active
        } else if state == AudioSessionStateInactive {
            SessionState::Inactive
        } else {
            return Err(DiscoveryError::InvalidData("unknown_session_state"));
        };
        let mut pid = 0;
        // SAFETY: live vtable and interface, valid initialized out pointer. Use the raw HRESULT
        // because the generated GetProcessId binding erases AUDCLNT_S_NO_SINGLE_PROCESS.
        let status =
            unsafe { (self.control.vtable().GetProcessId)(self.control.as_raw(), &mut pid) };
        status.ok().map_err(|error| native("GetProcessId", error))?;
        // SAFETY: HRESULT must be compared to S_OK; S_FALSE is success but means non-system.
        let system = unsafe { self.control.IsSystemSoundsSession() };
        system
            .ok()
            .map_err(|error| native("IsSystemSoundsSession", error))?;
        let scope = if system == S_OK {
            ProcessScope::SystemSounds
        } else if status == AUDCLNT_S_NO_SINGLE_PROCESS {
            ProcessScope::MultipleProcesses
        } else {
            ProcessScope::SingleProcess
        };
        if pid != self.pid {
            self.pid = pid;
            self.process_created = process::creation_time(pid);
            self.executable = process::executable(pid);
            self.icon_key = None;
            self.icon = None;
        }
        // SAFETY: both APIs transfer CoTaskMem-owned strings; take_string releases them.
        let name = unsafe {
            self.control
                .GetDisplayName()
                .ok()
                .and_then(|text| take_string(text).ok())
                .and_then(nonempty)
        };
        let icon_path = unsafe {
            self.control
                .GetIconPath()
                .ok()
                .and_then(|text| take_string(text).ok())
                .and_then(nonempty)
        };
        let icon_key = icon_path.or_else(|| self.executable.clone());
        if icon_key != self.icon_key {
            self.icon = icon_key
                .as_deref()
                .and_then(icon::extract)
                .or_else(|| self.executable.as_deref().and_then(icon::extract))
                .map(Arc::new);
            self.icon_key = icon_key;
        }
        let name = if scope == ProcessScope::SystemSounds {
            Some("系統音效".to_owned())
        } else {
            name.filter(|name| !name.starts_with('@')).or_else(|| {
                self.executable
                    .as_deref()
                    .and_then(process::executable_name)
            })
        };
        let meter: Option<IAudioMeterInformation> = self.control.cast().ok();
        let volume: Option<ISimpleAudioVolume> = self.control.cast().ok();
        // SAFETY: queried interfaces belong to the same live session, on the MTA worker.
        let (peak, level, muted) = unsafe {
            (
                meter
                    .and_then(|meter| meter.GetPeakValue().ok())
                    .and_then(normalized_level),
                volume
                    .as_ref()
                    .and_then(|volume| volume.GetMasterVolume().ok())
                    .and_then(normalized_level),
                volume
                    .and_then(|volume| volume.GetMute().ok())
                    .map(|muted| muted.as_bool()),
            )
        };
        Ok(Some(AudioSource {
            id: AudioSourceId {
                endpoint: endpoint.to_owned(),
                instance: instance.to_owned(),
            },
            endpoint_name: endpoint_name.clone(),
            process_id: pid,
            process_created: self.process_created,
            process_scope: scope,
            application_name: name.map(|name| name.chars().take(MAX_TITLE_CHARS).collect()),
            window_title: if scope == ProcessScope::SingleProcess {
                titles.get(&pid).cloned()
            } else {
                None
            },
            icon: self.icon.clone(),
            state,
            peak,
            volume: level,
            muted,
        }))
    }
}
