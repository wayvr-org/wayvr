#[cfg(feature = "feat-monado-metrics")]
use crate::subsystem::monado_metrics::{self, metrics_fd::MonadoMetricsFd};

pub struct MonadoState {
    pub ipc: libmonado::Monado,

    #[cfg(feature = "feat-monado-metrics")]
    pub metrics: Option<MonadoMetricsFd>,
}

impl MonadoState {
    pub fn new() -> anyhow::Result<Self> {
        let ipc = libmonado::Monado::auto_connect().map_err(|s| anyhow::anyhow!("{s}"))?;
        let res = Self {
            ipc,
            #[cfg(feature = "feat-monado-metrics")]
            metrics: None,
        };
        Ok(res)
    }

    #[allow(clippy::missing_const_for_fn)]
    #[allow(clippy::unused_self)]
    pub fn update(&mut self) {
        #[cfg(feature = "feat-monado-metrics")]
        if let Some(metrics) = &mut self.metrics {
            metrics.update();

            if metrics.is_full() {
                let _ = self.set_metrics_enabled(false); // disable metrics if they aren't used
            }
        }
    }

    #[allow(clippy::missing_const_for_fn)]
    #[allow(clippy::unused_self)]
    #[allow(clippy::unnecessary_wraps)]
    #[cfg(feature = "feat-monado-metrics")]
    pub fn set_metrics_enabled(&mut self, enabled: bool) -> anyhow::Result<()> {
        #[cfg(feature = "feat-monado-metrics")]
        {
            if enabled {
                if self.metrics.is_none() {
                    log::info!("Starting Monado metrics");
                    self.metrics = Some(monado_metrics::metrics_fd::MonadoMetricsFd::new(
                        &mut self.ipc,
                    )?);
                }
            } else {
                if self.metrics.is_some() {
                    log::info!("Stopping Monado metrics");
                }
                self.metrics = None;
            }
        }
        #[cfg(not(feature = "feat-monado-metrics"))]
        {
            #[allow(path_statements)]
            enabled;
        }

        Ok(())
    }
}
