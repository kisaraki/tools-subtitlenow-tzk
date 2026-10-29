//! Numeric resource sampling. Native queries belong behind ProcessProbe.
use std::{sync::Arc, time::Instant};
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResourceError {
    #[error("程序資源量測無法取得（{0:#010x}）。")]
    Native(i32),
    #[error("程序資源量測時間或計數不一致。")]
    InvalidSample,
}
#[derive(Clone, Copy, Debug)]
pub struct ProcessSample {
    pub at: Instant,
    pub created_100ns: u64,
    pub cpu_100ns: u64,
    pub private_commit_bytes: u64,
    pub working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
    pub handles: u32,
}
pub trait ProcessProbe: Send + Sync {
    fn sample(&self) -> Result<ProcessSample, ResourceError>;
}
#[derive(Clone, Copy, Debug)]
pub struct ProcessReading {
    pub sample: ProcessSample,
    pub cpu_core_ppm: Option<u64>,
    pub cpu_machine_ppm: Option<u64>,
}
pub struct ProcessSampler {
    probe: Arc<dyn ProcessProbe>,
    previous: Option<ProcessSample>,
    parallelism: usize,
}
impl ProcessSampler {
    pub fn new(probe: Arc<dyn ProcessProbe>) -> Self {
        Self {
            probe,
            previous: None,
            parallelism: std::thread::available_parallelism().map_or(1, usize::from),
        }
    }
    pub fn sample(&mut self) -> Result<ProcessReading, ResourceError> {
        let sample = self.probe.sample()?;
        let result = Self::delta(self.previous, sample, self.parallelism);
        self.previous = Some(sample);
        result
    }
    fn delta(
        previous: Option<ProcessSample>,
        sample: ProcessSample,
        parallelism: usize,
    ) -> Result<ProcessReading, ResourceError> {
        let core = if let Some(previous) = previous {
            if previous.created_100ns != sample.created_100ns {
                return Err(ResourceError::InvalidSample);
            }
            let cpu = sample
                .cpu_100ns
                .checked_sub(previous.cpu_100ns)
                .ok_or(ResourceError::InvalidSample)?;
            let elapsed = sample
                .at
                .checked_duration_since(previous.at)
                .filter(|d| !d.is_zero())
                .ok_or(ResourceError::InvalidSample)?
                .as_nanos();
            Some(
                u64::try_from(u128::from(cpu) * 100 * 1_000_000 / elapsed)
                    .map_err(|_| ResourceError::InvalidSample)?,
            )
        } else {
            None
        };
        Ok(ProcessReading {
            sample,
            cpu_core_ppm: core,
            cpu_machine_ppm: core.map(|v| v / parallelism.max(1) as u64),
        })
    }
}
#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
