//! Full functionality WS JSONRPC client implementation.
//! Follow <https://www.jsonrpc.org/specification> specification.

pub mod initializer;
pub use initializer::*;

// std
use std::{mem, time::Duration};
// crates.io
use futures::{
	future::{self, Either, Fuse},
	stream::{self, SplitSink, SplitStream},
	FutureExt, SinkExt, StreamExt,
};
use tokio::{net::TcpStream, sync::Mutex, time};
use tokio_stream::wrappers::IntervalStream;
use tokio_tungstenite::{
	tungstenite::{error::Result as WsResult, Message as WsMessage},
	MaybeTlsStream, WebSocketStream,
};
// subapeye
use crate::jsonrpc::{prelude::*, *};

type GenericConnect = Box<
	dyn FnOnce(
			Duration,
			WsSender,
			WsReceiver,
			MessageRx,
			ErrorTx,
			ExitRx,
		) -> Pin<Box<dyn Future<Output = ()> + Send>>
		+ Send,
>;

type WsSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;
type WsReceiver = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// A Ws instance.
///
/// Use this to interact with the server.
#[derive(Debug)]
pub struct Ws {
	messenger: MessageTx,
	request_queue: RequestQueue,
	request_timeout: Duration,
	reporter: Mutex<StdResult<ErrorRx, String>>,
	closer: Option<ExitTx>,
}
impl Ws {
	// Don't call this if code hasn't encountered any error yet,
	// as it will block the asynchronous process.
	async fn report(&self) -> Result<()> {
		let mut reporter = self.reporter.lock().await;
		let e = match mem::replace(
			&mut *reporter,
			Err("[jsonrpc::ws] temporary error placeholder".into()),
		) {
			Ok(r) => r
				.await
				.map_err(|_| error::almost_impossible(E_REPORTER_CHANNEL_CLOSED))?
				.to_string(),
			Err(e) => e,
		};

		*reporter = Err(e.clone());

		Err(error::Generic::Plain(e))?
	}

	async fn execute<F>(&self, future: F) -> Result<<F as Future>::Output>
	where
		F: Future,
	{
		Ok(time::timeout(self.request_timeout, future).await.map_err(error::Generic::Timeout)?)
	}
}
impl Drop for Ws {
	fn drop(&mut self) {
		if let Some(c) = self.closer.take() {
			let _ = c.send(());
		} else {
			//
		}
	}
}
#[async_trait::async_trait]
impl Jsonrpc for Ws {
	/// Send a single request.
	async fn request<'a, R>(&self, raw_request: R) -> Result<ResponseResult>
	where
		R: IntoRequestRaw<'a>,
	{
		let RequestQueueGuard { lock: id, .. } = self.request_queue.consume_once()?;
		let RequestRaw { method, params } = raw_request.into();
		let (tx, rx) = oneshot::channel();

		#[cfg(feature = "debug")]
		if self.messenger.send(Message::Debug(id)).await.is_err() {
			self.report().await?;
		}
		if self
			.messenger
			.send(Message::Request(Call {
				id,
				request: serde_json::to_string(&Request { jsonrpc: VERSION, id, method, params })
					.map_err(error::Generic::Serde)?,
				tx,
			}))
			.await
			.is_err()
		{
			self.report().await?;
		}
		if let Ok(r) = self.execute(rx).await? {
			r
		} else {
			self.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		}
	}

	/// Send a batch of requests.
	async fn batch<'a, R>(&self, raw_requests: Vec<R>) -> Result<Vec<ResponseResult>>
	where
		R: IntoRequestRaw<'a>,
	{
		if raw_requests.is_empty() {
			Err(error::Jsonrpc::EmptyBatch)?;
		}

		let RequestQueueGuard { lock: ids, .. } = self.request_queue.consume(raw_requests.len())?;
		let id = ids.first().ok_or(error::almost_impossible(E_EMPTY_LOCK))?.to_owned();
		let requests = ids
			.into_iter()
			.zip(raw_requests.into_iter())
			.map(|(id, raw_request)| {
				let RequestRaw { method, params } = raw_request.into();

				Request { jsonrpc: VERSION, id, method, params }
			})
			.collect::<Vec<_>>();
		let request = serde_json::to_string(&requests).map_err(error::Generic::Serde)?;
		let (tx, rx) = oneshot::channel();

		if self.messenger.send(Message::Batch(Call { id, request, tx })).await.is_err() {
			self.report().await?;
		}
		if let Ok(mut r) = self.execute(rx).await? {
			// Each id is unique.
			let _ = r.as_mut().map(|r| r.sort_unstable_by_key(|r| r.id()));

			r
		} else {
			self.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		}
	}

	async fn subscribe<'a, R, M, D>(
		&self,
		raw_request: R,
		unsubscribe_method: M,
	) -> Result<Subscriber<'a, Self, R, D>>
	where
		R: IntoRequestRaw<'a>,
		M: Send + AsRef<str>,
	{
		let id = self
			.request(raw_request)
			.await?
			.extract_err()?
			.result
			.as_str()
			.ok_or(error::Jsonrpc::InvalidSubscriptionId)?
			.to_owned();
		// TODO?: Configurable channel size.
		let (tx, rx) = mpsc::channel(self.request_queue.size);

		if self.messenger.send(Message::Subscription(Subscription { id, tx })).await.is_err() {
			self.report().await?;
		}

		todo!()
		// Ok(Subscriber { message_tx: tx, subscription_rx: rx })
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
impl FutureSelector {
	fn connector(&self) -> GenericConnect {
		Box::new(match self {
			FutureSelector::Futures => Self::connect_futures,
			FutureSelector::Tokio => Self::connect_tokio,
		})
	}

	fn connect_futures(
		interval: Duration,
		mut ws_tx: WsSender,
		mut ws_rx: WsReceiver,
		message_rx: MessageRx,
		error_tx: ErrorTx,
		exit_rx: ExitRx,
	) -> Pin<Box<dyn Future<Output = ()> + Send>> {
		Box::pin(async move {
			let message_rx =
				stream::unfold(message_rx, |mut r| async { r.recv().await.map(|m| (m, r)) });

			futures::pin_mut!(message_rx);

			let mut rxs_fut = future::select(message_rx.next(), ws_rx.next());
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
					Either::Left((
						Either::Left((maybe_message, ws_rx_next)),
						exit_or_interval_fut_,
					)) => {
						if !pool
							.on_message_ws(
								maybe_message.expect(E_MESSAGE_CHANNEL_CLOSED),
								&mut ws_tx,
							)
							.await
						{
							return;
						}

						rxs_fut = future::select(message_rx.next(), ws_rx_next);
						exit_or_interval_fut = exit_or_interval_fut_;
					},
					Either::Left((
						Either::Right((maybe_response, call_rx_next)),
						exit_or_interval_fut_,
					)) => {
						if let Some(response) = maybe_response {
							if let Err(e) = pool.on_response_ws(response).await {
								try_send(error_tx, e, true);

								return;
							}
						} else {
							try_send(error_tx, error::Websocket::Closed.into(), true);

							return;
						}

						rxs_fut = future::select(call_rx_next, ws_rx.next());
						exit_or_interval_fut = exit_or_interval_fut_;
					},
					Either::Right((Either::Left((_, _)), _)) => return,
					Either::Right((Either::Right((_, exit_rx)), rxs_fut_)) => {
						#[cfg(feature = "trace")]
						tracing::trace!("Tick(Ping)");

						if let Err(e) = ws_tx.send(WsMessage::Ping(Vec::new())).await {
							try_send(error_tx, error::Websocket::Tungstenite(e).into(), false);

							return;
						};

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
		mut message_rx: MessageRx,
		error_tx: ErrorTx,
		mut exit_rx: ExitRx,
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
					maybe_message = message_rx.recv() => {
						if !pool.on_message_ws(maybe_message.expect(E_MESSAGE_CHANNEL_CLOSED), &mut ws_tx).await {
							return;
						}
					},
					maybe_response = ws_rx.next() => {
						if let Some(response) = maybe_response {
							if let Err(e) = pool.on_response_ws(response).await {
								try_send(error_tx, e, true);

								return;
							}
						} else {
							try_send(error_tx, error::Websocket::Closed.into(), true);

							return;
						}
					}
					_ = &mut interval_fut => {
						#[cfg(feature = "trace")]
						tracing::trace!("Tick(Ping)");

						if let Err(e) = ws_tx.send(WsMessage::Ping(Vec::new())).await {
							try_send(error_tx, error::Websocket::Tungstenite(e).into(), false);

							return
						};

						interval_fut = interval_max.next().fuse();
					},
					_ = &mut exit_rx => {
						return;
					},
				}
			}
		})
	}
}
impl Default for FutureSelector {
	fn default() -> Self {
		Self::Tokio
	}
}

impl<T> Call<T>
where
	T: Debug,
{
	async fn try_send_ws(self, tx: &mut WsSender, pool: &mut Pool<Id, ResponseTx<T>>) -> bool {
		if let Err(e) = tx.send(WsMessage::Text(self.request)).await {
			try_send(self.tx, Err(error::Websocket::Tungstenite(e).into()), true)
		} else {
			pool.insert(self.id, self.tx);

			true
		}
	}
}

impl Pools {
	async fn on_message_ws(&mut self, message: Message, tx: &mut WsSender) -> bool {
		#[cfg(feature = "trace")]
		tracing::trace!("Message({message:?})");

		match message {
			#[cfg(feature = "debug")]
			Message::Debug(_) => {},
			Message::Request(c) =>
				if !c.try_send_ws(tx, &mut self.requests).await {
					return false;
				},
			Message::Batch(c) =>
				if !c.try_send_ws(tx, &mut self.batches).await {
					return false;
				},
			Message::Subscription(s) => {
				self.subscriptions.insert(s.id, s.tx);
			},
		}

		true
	}

	async fn on_response_ws(&mut self, response: WsResult<WsMessage>) -> Result<()> {
		#[cfg(feature = "trace")]
		tracing::trace!("Response({response:?})");

		match response {
			Ok(m) => match m {
				WsMessage::Binary(r) => self.process_response(&r).await,
				WsMessage::Text(r) => self.process_response(r.as_bytes()).await,
				WsMessage::Ping(_) => {
					tracing::trace!("ping");

					Ok(())
				},
				WsMessage::Pong(_) => {
					tracing::trace!("pong");

					Ok(())
				},
				WsMessage::Close(_) => {
					tracing::trace!("close");

					Ok(())
				},
				WsMessage::Frame(_) => {
					tracing::trace!("frame");

					Ok(())
				},
			},
			Err(e) => Err(error::Websocket::Tungstenite(e))?,
		}
	}
}
