use anyhow::Context as _;
use smol::{channel::bounded, io::AsyncWriteExt as _};
use std::{io::Read as _, path::Path, str, sync::OnceLock};

const IO_BUFFER_SIZE: usize = 256 * 1024;
const CHANNEL_CAPACITY: usize = 8;
const MAX_INITIAL_ALLOCATION: usize = 8 * 1024 * 1024;

pub struct HttpClientResponse {
	pub data: Vec<u8>,
}

impl HttpClientResponse {
	pub fn into_json<T>(self) -> anyhow::Result<T>
	where
		T: serde::de::DeserializeOwned,
	{
		let utf8 = str::from_utf8(&self.data)?;
		Ok(serde_json::from_str(utf8)?)
	}
}

#[derive(Debug, Clone, Copy)]
pub struct ProgressFuncData {
	pub bytes_downloaded: u64,
	pub file_size: u64,
}

pub type ProgressFunc<'a> = Box<dyn FnMut(ProgressFuncData) + 'a>;

pub struct GetParams<'a> {
	pub url: &'a str,
	pub on_progress: Option<ProgressFunc<'a>>,
}

enum HttpClientData {
	FileSize(u64),
	Chunk(Vec<u8>),
	Ended(anyhow::Result<()>),
}

struct DownloadStream {
	file_size: u64,
	receiver: smol::channel::Receiver<HttpClientData>,
}

static HTTP_AGENT: OnceLock<ureq::Agent> = OnceLock::new();

fn http_agent() -> &'static ureq::Agent {
	HTTP_AGENT.get_or_init(|| {
		ureq::Agent::config_builder()
			.max_redirects(10)
			.http_status_as_error(false)
			.build()
			.new_agent()
	})
}

fn thread_http_client(
	url: &str,
	allow_missing_content_length: bool,
	sender: smol::channel::Sender<HttpClientData>,
) -> anyhow::Result<()> {
	log::info!("fetching URL \"{}\"", url);
	let agent = http_agent().clone();

	let response = agent
		.get(url)
		.header("Accept-Encoding", "identity")
		.call()
		.with_context(|| format!("failed to fetch URL \"{url}\""))?;

	if !response.status().is_success() {
		anyhow::bail!("non-200 HTTP response: {}", response.status().as_u16(),);
	}

	let file_size = match response.body().content_length() {
		Some(file_size) => file_size,

		None if allow_missing_content_length => 0,

		None => {
			anyhow::bail!("HTTP response has no Content-Length header");
		}
	};

	sender.send_blocking(HttpClientData::FileSize(file_size))?;

	let mut reader = response.into_body().into_reader();
	let mut buffer = [0_u8; IO_BUFFER_SIZE];

	loop {
		let count = reader.read(&mut buffer).with_context(|| {
			format!(
				"failed while reading HTTP response body \
						 from \"{url}\""
			)
		})?;

		if count == 0 {
			break;
		}

		sender.send_blocking(HttpClientData::Chunk(buffer[..count].to_vec()))?;
	}

	Ok(())
}

/// Starts a HTTP request in a separate thread and streams its body through an async channel
/// The request, TLS operations, and all BodyReader reads remain inside one
/// blocking worker. Only owned byte chunks cross back to the async task.
async fn start_download(url: &str, allow_missing_content_length: bool) -> anyhow::Result<DownloadStream> {
	let url = url.to_owned();

	let (sender, receiver) = bounded::<HttpClientData>(CHANNEL_CAPACITY);
	std::thread::spawn(move || {
		let res = thread_http_client(&url, allow_missing_content_length, sender.clone());
		let _ = sender.send_blocking(HttpClientData::Ended(res));
	});

	let file_size = match receiver.recv().await? {
		HttpClientData::Ended(res) => match res {
			Ok(_) => anyhow::bail!("HTTP worker failure"),
			Err(e) => return Err(anyhow::anyhow!(e)),
		},
		HttpClientData::FileSize(file_size) => file_size,
		_ => {
			anyhow::bail!("HTTP worker failure")
		}
	};

	Ok(DownloadStream { file_size, receiver })
}

/// Downloads a response into memory.
///
/// This fails if the server does not provide a Content-Length header.
pub async fn get(mut params: GetParams<'_>) -> anyhow::Result<HttpClientResponse> {
	let DownloadStream { file_size, receiver } = start_download(params.url, false).await?;

	let initial_capacity = usize::try_from(file_size).unwrap_or(0).min(MAX_INITIAL_ALLOCATION);

	let mut data = Vec::with_capacity(initial_capacity);
	let mut bytes_downloaded = 0_u64;

	while let Ok(msg) = receiver.recv().await {
		match msg {
			HttpClientData::FileSize(_) => unreachable!(),
			HttpClientData::Ended(e) => {
				if let Err(e) = e {
					anyhow::bail!("HTTP request failed: {}", e);
				}
			}
			HttpClientData::Chunk(chunk) => {
				bytes_downloaded += chunk.len() as u64;
				data.extend_from_slice(&chunk);

				if let Some(on_progress) = params.on_progress.as_mut() {
					on_progress(ProgressFuncData {
						bytes_downloaded,
						file_size,
					});
				}
			}
		}
	}

	if bytes_downloaded != file_size {
		anyhow::bail!(
			"HTTP response size mismatch: expected {} bytes, received {}",
			file_size,
			bytes_downloaded,
		);
	}

	Ok(HttpClientResponse { data })
}

/// Downloads a response directly to a file.
///
/// Unlike `get`, this permits responses without a Content-Length header. In
/// that case, `ProgressFuncData::file_size` is zero.
///
/// An existing file is truncated. If the download fails after the file is
/// created, a partial file may remain at `path`.
pub async fn download_to_file(mut params: GetParams<'_>, path: impl AsRef<Path>) -> anyhow::Result<()> {
	let path = path.as_ref().to_owned();

	let DownloadStream { file_size, receiver } = start_download(params.url, true).await?;

	let mut file = smol::fs::File::create(&path)
		.await
		.with_context(|| format!("failed to create download file {:?}", path,))?;

	let mut bytes_downloaded = 0_u64;

	while let Ok(msg) = receiver.recv().await {
		match msg {
			HttpClientData::FileSize(_) => unreachable!(),
			HttpClientData::Ended(e) => {
				if let Err(e) = e {
					anyhow::bail!("HTTP request failed: {}", e);
				}
			}
			HttpClientData::Chunk(chunk) => {
				file
					.write_all(&chunk)
					.await
					.with_context(|| format!("failed to write download file {:?}", path,))?;

				bytes_downloaded += chunk.len() as u64;

				if let Some(on_progress) = params.on_progress.as_mut() {
					on_progress(ProgressFuncData {
						bytes_downloaded,
						file_size,
					});
				}
			}
		}
	}

	file
		.flush()
		.await
		.with_context(|| format!("failed to flush download file {:?}", path,))?;

	if file_size != 0 && bytes_downloaded != file_size {
		anyhow::bail!(
			"HTTP response size mismatch: expected {} bytes, received {}",
			file_size,
			bytes_downloaded,
		);
	}

	Ok(())
}

pub async fn get_simple(url: &str) -> anyhow::Result<HttpClientResponse> {
	get(GetParams { url, on_progress: None }).await
}
