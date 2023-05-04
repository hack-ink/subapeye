//!

pub use Initializer as WsInitializer;

// std
use std::{str, time::Duration};
// crates.io
use futures::StreamExt;
// subapeye
use crate::jsonrpc::{prelude::*, ws::*};

/// [`Ws`] initializer.
#[derive(Clone, Debug)]
pub struct Initializer<'a> {
	/// URI to connect to.
	///
	/// Default: `ws://127.0.0.1:9944`.
	pub uri: &'a str,
	/// Request pool's size.
	pub pool_size: Id,
	/// Send tick with this interval to keep the WS alive.
	pub interval: Duration,
	/// Request timeout.
	pub request_timeout: Duration,
	/// Future selector.
	pub future_selector: FutureSelector,
}
impl<'a> Initializer<'a> {
	/// Create a initializer with default configurations.
	pub fn new() -> Self {
		Self::default()
	}

	/// Set the [`uri`](#structfield.uri).
	pub fn uri(mut self, uri: &'a str) -> Self {
		self.uri = uri;

		self
	}

	/// Set the [`pool_size`](#structfield.pool_size).
	pub fn pool_size(mut self, pool_size: Id) -> Self {
		self.pool_size = pool_size;

		self
	}

	/// Set the [`interval`](#structfield.interval).
	pub fn interval(mut self, interval: Duration) -> Self {
		self.interval = interval;

		self
	}

	/// Set the [`request_timeout`](#structfield.request_timeout).
	pub fn request_timeout(mut self, request_timeout: Duration) -> Self {
		self.request_timeout = request_timeout;

		self
	}

	/// Set the [`future_selector`](#structfield.future_selector).
	pub fn future_selector(mut self, future_selector: FutureSelector) -> Self {
		self.future_selector = future_selector;

		self
	}

	/// Initialize the connection.
	pub async fn initialize(self) -> Result<Ws> {
		let (messenger, reporter, closer) = self.connect().await?;

		Ok(Ws {
			inner: Arc::new(WsInner {
				messenger,
				request_queue: RequestQueue::with_size(self.pool_size),
				request_timeout: self.request_timeout,
				reporter: Mutex::new(Ok(reporter)),
			}),
			closer: Some(closer),
		})
	}

	async fn connect(&self) -> Result<(MessageTx, ErrorRx, ExitTx)> {
		let connect_inner = self.future_selector.connector();
		let interval = self.interval;
		let (ws_tx, ws_rx) = tokio_tungstenite::connect_async(self.uri)
			.await
			.map_err(error::Websocket::Tungstenite)?
			.0
			.split();
		let (message_tx, message_rx) = mpsc::channel(self.pool_size);
		let (error_tx, error_rx) = oneshot::channel();
		let (exit_tx, exit_rx) = oneshot::channel();

		tokio::spawn(async move {
			connect_inner(interval, ws_tx, ws_rx, message_rx, error_tx, exit_rx).await
		});

		Ok((message_tx, error_rx, exit_tx))
	}
}
impl<'a> Default for Initializer<'a> {
	fn default() -> Self {
		Self {
			uri: "ws://127.0.0.1:9944",
			pool_size: 1_024,
			interval: Duration::from_secs(10),
			request_timeout: Duration::from_secs(30),
			future_selector: FutureSelector::default(),
		}
	}
}
