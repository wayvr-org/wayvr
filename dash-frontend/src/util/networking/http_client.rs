//
// example smol+hyper usage derived from
// https://github.com/smol-rs/smol/blob/master/examples/hyper-client.rs
// under Apache-2.0 + MIT license.
// Repository URL: https://github.com/smol-rs/smol
//

use anyhow::Context as _;
use async_native_tls::TlsStream;
use http_body_util::{BodyStream, Empty};
use hyper::Request;
use smol::{net::TcpStream, prelude::*};
use std::convert::TryInto;
use std::pin::Pin;
use std::task::{Context, Poll};
use wlx_common::async_executor::AsyncExecutor;
pub struct HttpClientResponse {
	pub data: Vec<u8>,
}

impl HttpClientResponse {
	pub fn as_json<T>(self) -> anyhow::Result<T>
	where
		T: for<'a> serde::Deserialize<'a>,
	{
		let utf8 = str::from_utf8(&self.data)?;
		Ok(serde_json::from_str::<T>(utf8)?)
	}
}

pub async fn get(executor: &AsyncExecutor, url: &str) -> anyhow::Result<HttpClientResponse> {
	log::info!("fetching URL \"{}\"", url);

	let url: hyper::Uri = url.try_into()?;
	let req = Request::builder()
		.header(
			hyper::header::HOST,
			url.authority().context("invalid authority")?.clone().as_str(),
		)
		.uri(url)
		.body(Empty::new())?;

	let resp = fetch(executor, req).await?;

	if !resp.status().is_success() {
		// non-200 HTTP response
		anyhow::bail!("non-200 HTTP response: {}", resp.status().as_str());
	}

	let body = BodyStream::new(resp.into_body())
		.try_fold(Vec::new(), |mut body, chunk| {
			if let Some(chunk) = chunk.data_ref() {
				body.extend_from_slice(chunk);
			}
			Ok(body)
		})
		.await?;

	Ok(HttpClientResponse { data: body })
}

async fn fetch(
	ex: &AsyncExecutor,
	req: hyper::Request<http_body_util::Empty<&'static [u8]>>,
) -> anyhow::Result<hyper::Response<hyper::body::Incoming>> {
	let io = {
		let host = req.uri().host().context("cannot parse host")?;

		match req.uri().scheme_str() {
			Some("http") => {
				let stream = {
					let port = req.uri().port_u16().unwrap_or(80);
					smol::net::TcpStream::connect((host, port)).await?
				};
				SmolStream::Plain(stream)
			}
			Some("https") => {
				// In case of HTTPS, establish a secure TLS connection first.
				let stream = {
					let port = req.uri().port_u16().unwrap_or(443);
					smol::net::TcpStream::connect((host, port)).await?
				};
				let stream = async_native_tls::connect(host, stream).await?;
				SmolStream::Tls(stream)
			}
			scheme => anyhow::bail!("unsupported scheme: {:?}", scheme),
		}
	};

	// Spawn the HTTP/1 connection.
	let (mut sender, conn) = hyper::client::conn::http1::handshake(smol_hyper::rt::FuturesIo::new(io)).await?;
	ex.spawn(async move {
		if let Err(e) = conn.await {
			println!("Connection failed: {:?}", e);
		}
	})
	.detach();

	// Get the result
	let result = sender.send_request(req).await?;
	Ok(result)
}

/// A TCP or TCP+TLS connection.
enum SmolStream {
	/// A plain TCP connection.
	Plain(TcpStream),

	/// A TCP connection secured by TLS.
	Tls(TlsStream<TcpStream>),
}

impl AsyncRead for SmolStream {
	fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<smol::io::Result<usize>> {
		match &mut *self {
			SmolStream::Plain(stream) => Pin::new(stream).poll_read(cx, buf),
			SmolStream::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
		}
	}
}

impl AsyncWrite for SmolStream {
	fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<smol::io::Result<usize>> {
		match &mut *self {
			SmolStream::Plain(stream) => Pin::new(stream).poll_write(cx, buf),
			SmolStream::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
		}
	}

	fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<smol::io::Result<()>> {
		match &mut *self {
			SmolStream::Plain(stream) => Pin::new(stream).poll_close(cx),
			SmolStream::Tls(stream) => Pin::new(stream).poll_close(cx),
		}
	}

	fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<smol::io::Result<()>> {
		match &mut *self {
			SmolStream::Plain(stream) => Pin::new(stream).poll_flush(cx),
			SmolStream::Tls(stream) => Pin::new(stream).poll_flush(cx),
		}
	}
}
