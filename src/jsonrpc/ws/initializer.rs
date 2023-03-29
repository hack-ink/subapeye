//!

pub use Initializer as WsInitializer;

// std
use std::{future::Future, pin::Pin, str, time::Duration};
// crates.io
use futures::{
	future::{self, Either, Fuse},
	stream, FutureExt, StreamExt,
};
use tokio_stream::wrappers::IntervalStream;
// subapeye
use crate::{jsonrpc::ws::*, prelude::*};

type GenericConnect = Box<
	dyn FnOnce(
			Duration,
			WsSender,
			WsReceiver,
			CallReceiver,
			ErrorSender,
			ExitReceiver,
		) -> Pin<Box<dyn Future<Output = ()> + Send>>
		+ Send,
>;

/// [`Ws`] initializer.
#[derive(Clone, Debug)]
pub struct Initializer<'a> {
	/// URI to connect to.
	///
	/// Default: `ws://127.0.0.1:9944`.
	pub uri: &'a str,
	/// Maximum concurrent task count.
	pub max_concurrency: Id,
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

	/// Set the [`max_concurrency`](#structfield.max_concurrency).
	pub fn max_concurrency(mut self, max_concurrency: Id) -> Self {
		self.max_concurrency = max_concurrency;

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
		let (messenger, reporter, closer) = self
			.connect(match self.future_selector {
				FutureSelector::Futures => Box::new(connect_futures),
				FutureSelector::Tokio => Box::new(connect_tokio),
			})
			.await?;

		Ok(Ws {
			messenger,
			request_queue: RequestQueue {
				size: self.max_concurrency,
				active: Arc::new(()),
				// Id 0 is reserved for system health check.
				next: AtomicUsize::new(1),
			},
			request_timeout: self.request_timeout,
			reporter: Some(reporter),
			closer: Some(closer),
		})
	}

	async fn connect(
		&self,
		connect_inner: GenericConnect,
	) -> Result<(CallSender, ErrorReceiver, ExitSender)> {
		let interval = self.interval;
		let (ws_tx, ws_rx) = tokio_tungstenite::connect_async(self.uri).await?.0.split();
		let (call_tx, call_rx) = mpsc::channel(self.max_concurrency);
		let (error_tx, error_rx) = oneshot::channel();
		let (exit_tx, exit_rx) = oneshot::channel();

		tokio::spawn(async move {
			connect_inner(interval, ws_tx, ws_rx, call_rx, error_tx, exit_rx).await
		});

		Ok((call_tx, error_rx, exit_tx))
	}
}
impl<'a> Default for Initializer<'a> {
	fn default() -> Self {
		Self {
			uri: "ws://127.0.0.1:9944",
			max_concurrency: num_cpus::get(),
			interval: Duration::from_secs(10),
			request_timeout: Duration::from_secs(30),
			future_selector: FutureSelector::default(),
		}
	}
}

/// Async future selectors.
#[derive(Clone, Debug)]
pub enum FutureSelector {
	/// Use [`futures::future::select`].
	Futures,
	/// Use [`tokio::select!`].
	Tokio,
}
impl Default for FutureSelector {
	fn default() -> Self {
		Self::Tokio
	}
}

fn connect_futures(
	interval: Duration,
	mut ws_tx: WsSender,
	mut ws_rx: WsReceiver,
	call_rx: CallReceiver,
	error_tx: ErrorSender,
	exit_rx: ExitReceiver,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
	Box::pin(async move {
		let call_rx = stream::unfold(call_rx, |mut r| async { r.recv().await.map(|c| (c, r)) });

		futures::pin_mut!(call_rx);

		let mut rxs_fut = future::select(call_rx.next(), ws_rx.next());
		// TODO: clean dead items?
		let mut pool = Pools::new();
		// Minimum interval is 1ms.
		let interval_max = interval.max(Duration::from_millis(1));
		let mut interval_max = IntervalStream::new(time::interval(interval_max));
		// Disable the tick, if the interval is zero.
		let mut exit_or_interval_fut = future::select(
			exit_rx,
			if interval.is_zero() { Fuse::terminated() } else { interval_max.next().fuse() },
		);

		loop {
			match future::select(rxs_fut, exit_or_interval_fut).await {
				Either::Left((Either::Left((maybe_call, ws_rx_next)), exit_or_interval_fut_)) => {
					if let Some(c) = maybe_call {
						#[cfg(feature = "trace")]
						tracing::trace!("Call({c:?})");

						if !c.try_send(&mut ws_tx, &mut pool).await {
							return;
						}
					} else {
						try_send(error_tx, error::Tokio::ChannelClosed.into());

						return;
					}

					rxs_fut = future::select(call_rx.next(), ws_rx_next);
					exit_or_interval_fut = exit_or_interval_fut_;
				},
				Either::Left((
					Either::Right((maybe_response, call_rx_next)),
					exit_or_interval_fut_,
				)) => {
					if let Some(response) = maybe_response {
						pool.on_ws_recv(response).await.unwrap()
					} else {
						// TODO?: closed
					}

					rxs_fut = future::select(call_rx_next, ws_rx.next());
					exit_or_interval_fut = exit_or_interval_fut_;
				},
				Either::Right((Either::Left((_, _)), _)) => return,
				Either::Right((Either::Right((_, exit_rx)), rxs_fut_)) => {
					#[cfg(feature = "trace")]
					tracing::trace!("TickRequest(Ping)");

					ws_tx.send(Message::Text("Ping".into())).await.unwrap();

					rxs_fut = rxs_fut_;
					exit_or_interval_fut = future::select(
						exit_rx,
						if interval.is_zero() {
							Fuse::terminated()
						} else {
							interval_max.next().fuse()
						},
					);
				},
			}
		}
	})
}

fn connect_tokio(
	interval: Duration,
	mut ws_tx: WsSender,
	mut ws_rx: WsReceiver,
	mut call_rx: CallReceiver,
	error_tx: ErrorSender,
	mut exit_rx: ExitReceiver,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
	Box::pin(async move {
		// TODO: clean dead items?
		let mut pool = Pools::new();
		// Minimum interval is 1ms.
		let interval_max = interval.max(Duration::from_millis(1));
		let mut interval_max = IntervalStream::new(time::interval(interval_max));
		// Disable the tick, if the interval is zero.
		let mut interval_fut =
			if interval.is_zero() { Fuse::terminated() } else { interval_max.next().fuse() };

		loop {
			tokio::select! {
				maybe_request = call_rx.recv() => {
					if let Some(c) = maybe_request {
						#[cfg(feature = "trace")]
						tracing::trace!("Call({c:?})");

						if !c.try_send(&mut ws_tx, &mut pool).await {
							return;
						}
					} else {
						try_send(error_tx, error::Tokio::ChannelClosed.into());

						return;
					}
				},
				maybe_response = ws_rx.next() => {
					if let Some(response) = maybe_response {
						pool.on_ws_recv(response).await.unwrap()
					} else {
						// TODO?: closed
					}
				}
				_ = &mut interval_fut => {
					#[cfg(feature = "trace")]
					tracing::trace!("TickRequest(Ping)");

					ws_tx.send(Message::Ping(Vec::new())).await.unwrap();

					interval_fut = interval_max.next().fuse();
				},
				_ = &mut exit_rx => {
					return;
				},
			}
		}
	})
}
