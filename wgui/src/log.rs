use std::fmt::Debug;

pub trait LogErr {
	#[must_use]
	fn log_err(self, additional: &str) -> Self;
	#[must_use]
	fn log_err_with<D: Debug>(self, additional: &D) -> Self;
	#[must_use]
	fn log_warn(self, additional: &str) -> Self;
	#[must_use]
	fn log_warn_with<D: Debug>(self, additional: &D) -> Self;
}

impl<T, E> LogErr for Result<T, E>
where
	E: Debug + Send + Sync + 'static,
{
	fn log_warn(self, additional: &str) -> Result<T, E> {
		match self {
			Ok(ok) => Ok(ok),
			Err(error) => {
				log::warn!("{additional}: {error:?}");
				Err(error)
			}
		}
	}

	fn log_warn_with<D: Debug>(self, additional: &D) -> Result<T, E> {
		match self {
			Ok(ok) => Ok(ok),
			Err(error) => {
				log::warn!("{additional:?}: {error:?}");
				Err(error)
			}
		}
	}

	fn log_err(self, additional: &str) -> Result<T, E> {
		match self {
			Ok(ok) => Ok(ok),
			Err(error) => {
				log::error!("{additional}: {error:?}");
				Err(error)
			}
		}
	}

	fn log_err_with<D: Debug>(self, additional: &D) -> Self {
		match self {
			Ok(ok) => Ok(ok),
			Err(error) => {
				log::error!("{additional:?}: {error:?}");
				Err(error)
			}
		}
	}
}
