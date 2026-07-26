use std::sync::{Arc, Mutex as SyncMutex};

pub type Serial = u64;

#[derive(Clone, Default)]
pub struct SerialGenerator {
	serial: Arc<SyncMutex<u64>>,
}

impl SerialGenerator {
	pub fn new() -> SerialGenerator {
		Self {
			serial: Arc::new(SyncMutex::new(0)),
		}
	}

	pub fn increment_get(&self) -> Serial {
		let mut serial = self.serial.lock().unwrap();
		let cur = *serial;
		*serial += 1;
		cur
	}
}

pub const PROTOCOL_VERSION: u32 = 4;
pub const CONNECTION_MAGIC: &str = "wayvr_ipc";

pub fn data_encode<T>(data: &T) -> Vec<u8>
where
	T: serde::Serialize,
{
	let str = serde_json::to_string(&data).unwrap();
	log::debug!("serialized data: {}", str);
	str.into_bytes()
}

pub fn data_decode<T>(data: &[u8]) -> anyhow::Result<T>
where
	T: for<'a> serde::Deserialize<'a>,
{
	Ok(serde_json::from_str::<T>(std::str::from_utf8(data)?)?)
}
