use crate::subsystem::monado_metrics::{self, metrics_fd::MonadoMetricsFd};

pub struct MonadoState {
    pub ipc: libmonado::Monado,
    pub metrics: Option<MonadoMetricsFd>,
}

impl MonadoState {
    pub fn new() -> anyhow::Result<Self> {
        let mut ipc = libmonado::Monado::auto_connect().map_err(|s| anyhow::anyhow!("{s}"))?;

        let metrics = monado_metrics::metrics_fd::MonadoMetricsFd::new(&mut ipc)?;

        Ok(Self {
            ipc,
            metrics: Some(metrics),
        })
    }

    pub fn update(&mut self) {
        if let Some(metrics) = &mut self.metrics {
            metrics.update();
        }
    }
}
