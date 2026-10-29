//! Local fingerprint → explicitly supplied catalog → normal resolver evidence.
//! No catalog/network endpoint is selected by default, and no match is synthesized.
use super::{
    contracts::*,
    fingerprint::ChromaprintExtractor,
    fingerprint_worker::{FingerprintClient, FingerprintWorker},
};
use crate::ai::speech::StreamIdentity;
use std::{sync::Arc, time::Instant};

#[derive(Clone, Copy)]
pub struct CatalogCapabilities {
    pub format: FingerprintFormat,
    pub cancellation: bool,
}
pub trait FingerprintCatalog: Send + Sync {
    fn capabilities(&self) -> CatalogCapabilities;
    /// Implementations must bound responses, honor cancellation/Drop and use an
    /// explicitly configured secure transport. Identifying content never adopts text.
    fn lookup<'a>(
        &'a self,
        fingerprint: &'a Fingerprint,
        stamp: WindowStamp,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a>;
}
pub struct CatalogIdentifier {
    identity: StreamIdentity,
    worker: FingerprintWorker,
    client: FingerprintClient,
    catalog: Arc<dyn FingerprintCatalog>,
}
impl CatalogIdentifier {
    pub fn new(
        identity: StreamIdentity,
        catalog: Arc<dyn FingerprintCatalog>,
    ) -> Result<Self, ResolverError> {
        let capabilities = catalog.capabilities();
        if capabilities.format != FingerprintFormat::ChromaprintTest2Compressed
            || !capabilities.cancellation
        {
            return Err(ResolverError::UnsupportedCapability);
        }
        let worker = FingerprintWorker::start(identity, Arc::new(ChromaprintExtractor))?;
        let client = worker.client();
        Ok(Self {
            identity,
            worker,
            client,
            catalog,
        })
    }
    pub fn worker_count(&self) -> usize {
        self.worker.worker_count()
    }
}
impl ContentIdentifier for CatalogIdentifier {
    fn capabilities(&self) -> ResolverCapabilities {
        ResolverCapabilities {
            classification: false,
            songs: true,
            movies: false,
            cancellation: true,
        }
    }
    fn identify<'a>(
        &'a self,
        window: &'a AudioWindow,
        cancel: ResolverCancellation,
    ) -> IdentifyFuture<'a> {
        Box::pin(async move {
            if cancel.is_cancelled() {
                return Err(ResolverError::Cancelled);
            }
            window.stamp.validate(self.identity, Instant::now())?;
            // Only clone the immutable PCM Arc. The caller's window data is not copied
            // into another audio queue and the non-Send Fingerprinter stays on its worker.
            let owned = Arc::new(AudioWindow {
                stamp: window.stamp,
                pcm: window.pcm.clone(),
            });
            let job = self.client.request(owned)?;
            let result = tokio::select! { biased;
                _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
                result = job.result() => result?,
            };
            let evidence = tokio::select! { biased;
                _ = cancel.cancelled() => return Err(ResolverError::Cancelled),
                result = self.catalog.lookup(&result.fingerprint, result.stamp, cancel.clone()) => result?,
            };
            result.stamp.validate(self.identity, Instant::now())?;
            evidence.validate()?;
            if !self.capabilities().accepts(&evidence) {
                return Err(ResolverError::UnsupportedCapability);
            }
            Ok(evidence)
        })
    }
}
