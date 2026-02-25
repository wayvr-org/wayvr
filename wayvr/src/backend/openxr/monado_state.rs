use crate::subsystem::monado_metrics::{self, metrics_fd::MonadoMetricsFd};

pub struct MonadoState {
    pub ipc: libmonado::Monado,
    pub metrics: Option<MonadoMetricsFd>,
}

impl MonadoState {
    pub fn new() -> anyhow::Result<Self> {
        let ipc = libmonado::Monado::auto_connect().map_err(|s| anyhow::anyhow!("{s}"))?;
        let mut res = Self { ipc, metrics: None };
        res.set_metrics_enabled(true)?;
        Ok(res)
    }

    pub fn update(&mut self) {
        if let Some(metrics) = &mut self.metrics {
            metrics.update();
        }
    }

    pub fn set_metrics_enabled(&mut self, enabled: bool) -> anyhow::Result<()> {
        if enabled && self.metrics.is_none() {
            self.metrics = Some(monado_metrics::metrics_fd::MonadoMetricsFd::new(
                &mut self.ipc,
            )?);
        } else {
            self.metrics = None;
        }

        Ok(())
    }
}
