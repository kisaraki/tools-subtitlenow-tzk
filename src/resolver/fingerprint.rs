//! Real, local Chromaprint test2 extraction. No catalog lookup, audio storage or identity guesses.
use super::{
    contracts::*,
    window::{WINDOW_FRAMES, WINDOW_SECONDS},
};
use rusty_chromaprint::{Configuration, FingerprintCompressor, Fingerprinter};
use std::time::Instant;

/// Check cancellation between 100 ms input slices. The library internally batches
/// FFT/resampling, so this is not a guaranteed 100 ms cancellation latency.
const FEED_SAMPLES: usize = AudioWindow::SAMPLE_RATE as usize / 10;
/// Gate only unusable/near-silent audio, not semantic Speech/Music classification.
const MIN_RMS_PCM: f64 = 16.0;
pub struct ChromaprintExtractor;
impl FingerprintExtractor for ChromaprintExtractor {
    fn extract(
        &self,
        input: &AudioWindow,
        cancel: &ResolverCancellation,
    ) -> Result<Fingerprint, ResolverError> {
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        input.stamp.validate(input.stamp.identity, Instant::now())?;
        if input.samples().len() != WINDOW_FRAMES
            || input.stamp.duration_us != WINDOW_SECONDS as u64 * 1_000_000
        {
            return Err(ResolverError::InvalidData);
        }
        let mut energy = 0f64;
        for chunk in input.samples().chunks(FEED_SAMPLES) {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            energy += chunk
                .iter()
                .map(|sample| f64::from(*sample).powi(2))
                .sum::<f64>();
        }
        if (energy / input.samples().len() as f64).sqrt() < MIN_RMS_PCM {
            return Err(ResolverError::InsufficientSignal);
        }
        // A new non-Send Fingerprinter is constructed, used and destroyed on the CPU
        // worker for each bounded window; it never crosses threads or enters async tasks.
        let config = Configuration::preset_test2();
        let mut printer = Fingerprinter::new(&config);
        printer
            .start(AudioWindow::SAMPLE_RATE, 1)
            .map_err(|_| ResolverError::InvalidData)?;
        for chunk in input.samples().chunks(FEED_SAMPLES) {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            printer.consume(chunk);
        }
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        printer.finish();
        if cancel.is_cancelled() {
            return Err(ResolverError::Cancelled);
        }
        input.stamp.validate(input.stamp.identity, Instant::now())?;
        let values = printer.fingerprint();
        if values.is_empty() {
            return Err(ResolverError::InsufficientSignal);
        }
        if values.len() > MAX_FINGERPRINT_BYTES / std::mem::size_of::<u32>() {
            return Err(ResolverError::InvalidData);
        }
        let compressed = FingerprintCompressor::from(&config).compress(values);
        Fingerprint::chromaprint(compressed.into_boxed_slice())
    }
}
