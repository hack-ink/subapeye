//! Full functionality WS JSONRPC client implementation.
//! Follow <https://www.jsonrpc.org/specification> specification.

pub mod initializer;
pub use initializer::*;

// std
use std::task::{Context, Poll};
// crates.io
use futures::{
	future::{self, Either, Fuse},
	stream::{self, SplitSink, SplitStream},
	FutureExt, SinkExt, Stream, StreamExt,
};
use serde::de::DeserializeOwned;
use tokio::net::TcpStream;
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
			MessageExtRx,
			ErrorTx,
			ExitRx,
		) -> Pin<Box<dyn Future<Output = ()> + Send>>
		+ Send,
>;

type WsSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>;
type WsReceiver = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

type MessageExtTx = mpsc::Sender<MessageExt>;
type MessageExtRx = mpsc::Receiver<MessageExt>;

type NotificationTx = mpsc::Sender<SubscriptionResult>;
type NotificationRx = mpsc::Receiver<SubscriptionResult>;

type ResultTx = oneshot::Sender<Result<()>>;
type SubscriptionIdTx = oneshot::Sender<SubscriptionId>;

type SubscriptionPool = Pool<Id, (NotificationTx, SubscriptionIdTx)>;
type NotificationPool = Pool<SubscriptionId, NotificationTx>;

const E_INVALID_SUBSCRIPTION_ID: &str = "[jsonrpc::ws] invalid subscription id";
const E_SUBSCRIPTION_ID_CHANNEL_CLOSED: &str = "[jsonrpc::ws] subscription id channel closed";

/// A Ws instance.
///
/// Use this to interact with the server.
#[derive(Debug)]
pub struct Ws {
	message_tx: MessageExtTx,
	request_queue: RequestQueue,
	timeout: Duration,
	reporter: Reporter,
	exit_tx: Option<ExitTx>,
}
impl Ws {
	async fn execute<F>(&self, future: F) -> Result<<F as Future>::Output>
	where
		F: Future,
	{
		execute(future, self.timeout).await
	}
}
impl Drop for Ws {
	fn drop(&mut self) {
		if let Some(c) = self.exit_tx.take() {
			let _ = c.send(());
		} else {
			//
		}
	}
}
#[async_trait::async_trait]
impl Jsonrpc for Ws {
	async fn request<'a, R>(&self, request_raw: R) -> Result<ResponseResult>
	where
		R: IntoRequestRaw<&'a str>,
	{
		let RequestQueueGuard { lock: id, .. } = self.request_queue.consume_once()?;
		let RequestRaw { method, params } = request_raw.into();
		let (tx, rx) = oneshot::channel();

		#[cfg(feature = "debug")]
		if self.message_tx.send(MessageExt::Debug(id)).await.is_err() {
			self.reporter.report().await?;
		}
		if self
			.message_tx
			.send(MessageExt::Message(Message::Request(CallOnce {
				id,
				request: serde_json::to_string(&Request { jsonrpc: VERSION, id, method, params })
					.map_err(error::Generic::Serde)?,
				tx,
			})))
			.await
			.is_err()
		{
			self.reporter.report().await?;
		}
		if let Ok(r) = self.execute(rx).await? {
			r
		} else {
			self.reporter.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		}
	}

	async fn batch<'a, R>(&self, requests_raw: Vec<R>) -> Result<Vec<ResponseResult>>
	where
		R: IntoRequestRaw<&'a str>,
	{
		if requests_raw.is_empty() {
			Err(error::Jsonrpc::EmptyBatch)?;
		}

		let RequestQueueGuard { lock: ids, .. } =
			self.request_queue.consume(requests_raw.len() + 1)?;

		assert_eq!(requests_raw.len(), ids.len(), "{}", E_UNEXPECTED_LOCK_COUNT);

		let id = ids.get(0).expect(E_UNEXPECTED_LOCK_COUNT).to_owned();
		let requests = ids
			.into_iter()
			.skip(1)
			.zip(requests_raw.into_iter())
			.map(|(id, request_raw)| {
				let RequestRaw { method, params } = request_raw.into();

				Request { jsonrpc: VERSION, id, method, params }
			})
			.collect::<Vec<_>>();
		let request = serde_json::to_string(&requests).map_err(error::Generic::Serde)?;
		let (tx, rx) = oneshot::channel();

		if self
			.message_tx
			.send(MessageExt::Message(Message::Batch(CallOnce { id, request, tx })))
			.await
			.is_err()
		{
			self.reporter.report().await?;
		}
		if let Ok(mut r) = self.execute(rx).await? {
			// Each id is unique.
			let _ = r.as_mut().map(|r| r.sort_unstable_by_key(|r| r.id()));

			r
		} else {
			self.reporter.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		}
	}
}
#[async_trait::async_trait]
impl JsonrpcExt for Ws {
	async fn subscribe<'a, R, D>(
		&self,
		request_raw: R,
		unsubscribe_method: String,
	) -> Result<Subscriber<D>>
	where
		R: IntoRequestRaw<&'a str>,
	{
		let RequestQueueGuard { lock: ids, .. } = self.request_queue.consume(2)?;
		let id = ids.get(0).expect(E_UNEXPECTED_LOCK_COUNT).to_owned();
		let unsubscribe_id = ids.get(1).expect(E_UNEXPECTED_LOCK_COUNT).to_owned();
		let RequestRaw { method, params } = request_raw.into();
		// TODO?: Configurable channel size.
		let (notification_tx, notification_rx) = mpsc::channel(self.request_queue.size);
		let (result_tx, result_rx) = oneshot::channel();
		let (subscription_tx, subscription_rx) = oneshot::channel();

		if self
			.message_tx
			.send(MessageExt::Subscribe(Call {
				id,
				request: serde_json::to_string(&Request { jsonrpc: VERSION, id, method, params })
					.map_err(error::Generic::Serde)?,
				notification_tx,
				result_tx,
				subscription_tx,
			}))
			.await
			.is_err()
		{
			self.reporter.report().await?;
		}
		if let Ok(r) = self.execute(result_rx).await? {
			r?;
		} else {
			self.reporter.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?;
		}

		let subscription_id = if let Ok(r) = self.execute(subscription_rx).await? {
			r
		} else {
			self.reporter.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		};

		Ok(Subscriber {
			message_tx: self.message_tx.clone(),
			id,
			subscription_id,
			notification_rx,
			unsubscribe_id,
			unsubscribe_method,
			reporter: self.reporter.clone(),
			timeout: self.timeout.clone(),
			_deserialize: Default::default(),
		})
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
		message_rx: MessageExtRx,
		error_tx: ErrorTx,
		exit_rx: ExitRx,
	) -> Pin<Box<dyn Future<Output = ()> + Send>> {
		Box::pin(async move {
			let message_rx =
				stream::unfold(message_rx, |mut r| async { r.recv().await.map(|m| (m, r)) });

			futures::pin_mut!(message_rx);

			let mut rxs_fut = future::select(message_rx.next(), ws_rx.next());
			// TODO: clean dead items?
			let mut pool = PoolsExt::new();
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
							.on_message(maybe_message.expect(E_MESSAGE_CHANNEL_CLOSED), &mut ws_tx)
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
							if let Err(e) = pool.on_response(response).await {
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
		mut message_rx: MessageExtRx,
		error_tx: ErrorTx,
		mut exit_rx: ExitRx,
	) -> Pin<Box<dyn Future<Output = ()> + Send>> {
		Box::pin(async move {
			// TODO: clean dead items?
			let mut pool = PoolsExt::new();
			// Minimum interval is 1ms.
			let interval_max = interval.max(Duration::from_millis(1));
			let mut interval_max = IntervalStream::new(time::interval(interval_max));
			// Disable the tick, if the interval is zero.
			let mut interval_fut =
				if interval.is_zero() { Fuse::terminated() } else { interval_max.next().fuse() };

			loop {
				tokio::select! {
					maybe_message = message_rx.recv() => {
						if !pool.on_message(maybe_message.expect(E_MESSAGE_CHANNEL_CLOSED), &mut ws_tx).await {
							return;
						}
					},
					maybe_response = ws_rx.next() => {
						if let Some(response) = maybe_response {
							if let Err(e) = pool.on_response(response).await {
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

///
#[derive(Debug)]
pub struct Subscriber<D> {
	message_tx: MessageExtTx,
	id: Id,
	subscription_id: SubscriptionId,
	notification_rx: NotificationRx,
	unsubscribe_id: Id,
	unsubscribe_method: String,
	reporter: Reporter,
	///
	pub timeout: Duration,
	_deserialize: PhantomData<D>,
}
impl<D> Subscriber<D> {
	///
	pub fn timeout(&mut self, timeout: Duration) -> &mut Self {
		self.timeout = timeout;

		self
	}

	///
	pub async fn unsubscribe(&self) -> Result<ResponseResult> {
		self.message_tx
			.send(MessageExt::Unsubscribe((self.id, self.subscription_id.clone())))
			.await
			.map_err(|_| error::almost_impossible(E_MESSAGE_CHANNEL_CLOSED))?;
		let (tx, rx) = oneshot::channel();
		let id = self.unsubscribe_id;

		if self
			.message_tx
			.send(MessageExt::Message(Message::Request(CallOnce {
				id,
				request: serde_json::to_string(&Request {
					jsonrpc: VERSION,
					id,
					method: &self.unsubscribe_method,
					params: [[&self.subscription_id]],
				})
				.map_err(error::Generic::Serde)?,

				tx,
			})))
			.await
			.is_err()
		{
			self.reporter.report().await;
		}
		if let Ok(r) = execute(rx, self.timeout).await? {
			r
		} else {
			self.reporter.report().await.and(Err(error::almost_impossible(E_NO_ERROR))?)?
		}
	}
}
impl<D> Subscriber<D>
where
	D: Unpin,
{
	///
	pub async fn next_raw(&mut self) -> Result<Option<SubscriptionResult>> {
		let timeout = self.timeout.clone();

		execute(StreamExt::next(self), timeout).await
	}
}
impl<D> Subscriber<D>
where
	D: DeserializeOwned + Unpin,
{
	///
	pub async fn next(&mut self) -> Result<Option<Result<D>>> {
		Ok(self.next_raw().await?.map(|r| {
			r.map_err(|e| error::Error::Jsonrpc(error::Jsonrpc::Response(e.error))).and_then(|o| {
				Ok(serde_json::from_value(o.params.result).map_err(error::Generic::Serde)?)
			})
		}))
	}
}
impl<D> Stream for Subscriber<D>
where
	D: Unpin,
{
	type Item = SubscriptionResult;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
		self.notification_rx.poll_recv(cx)
	}
}

#[derive(Debug, Default)]
struct PoolsExt {
	regular_pools: Pools,
	subscriptions: SubscriptionPool,
	notifications: NotificationPool,
}
impl PoolsExt {
	fn new() -> Self {
		Default::default()
	}

	async fn process_response(&mut self, response: &[u8]) -> Result<()> {
		let r = response.trim_ascii_start();
		let first = r.first().ok_or(error::Jsonrpc::EmptyResponse)?;

		match first {
			b'{' =>
				if let Ok(o) = serde_json::from_slice::<ResponseOk>(r) {
					if let Some((notification, subscription_tx)) = self.subscriptions.remove(&o.id)
					{
						let subscription_id = o
							.result
							.as_str()
							.ok_or(error::almost_impossible(E_INVALID_SUBSCRIPTION_ID))?
							.to_owned();

						subscription_tx
							.send(subscription_id.clone())
							.expect(E_SUBSCRIPTION_ID_CHANNEL_CLOSED);

						self.notifications.insert(subscription_id, notification);
					} else {
						self.regular_pools
							.requests
							.take_tx(&o.id)
							.send(Ok(Ok(o)))
							.expect(E_RESPONSE_CHANNEL_CLOSED);
					}

					return Ok(());
				} else if let Ok(e) = serde_json::from_slice::<JsonrpcError>(r) {
					// E.g.
					// ```
					// {"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":2}
					// ```

					self.regular_pools
						.requests
						.take_tx(&e.id)
						.send(Ok(Err(e)))
						.expect(E_RESPONSE_CHANNEL_CLOSED);

					return Ok(());
				} else if let Ok(o) = serde_json::from_slice::<NotificationOk>(r) {
					self.notifications
						.get_tx(&o.params.subscription)
						.send(Ok(o))
						.await
						.expect(E_RESPONSE_CHANNEL_CLOSED);

					return Ok(());
				},
			b'[' =>
				if let Ok(r) = serde_json::from_slice::<Vec<&RawValue>>(r) {
					let r = r
						.into_iter()
						.map(|r| {
							if let Ok(o) = serde_json::from_str::<ResponseOk>(r.get()) {
								Ok(Ok(o))
							} else if let Ok(e) = serde_json::from_str::<JsonrpcError>(r.get()) {
								Ok(Err(e))
							} else {
								Err(error::almost_impossible(E_INVALID_RESPONSE))?
							}
						})
						.collect::<Result<Vec<ResponseResult>>>()?;

					self.regular_pools
						.batches
						.take_tx(&r.first().ok_or(error::Jsonrpc::EmptyBatch)?.id())
						.send(Ok(r))
						.expect(E_RESPONSE_CHANNEL_CLOSED);

					return Ok(());
				},
			_ => (),
		}

		Err(error::almost_impossible(E_INVALID_RESPONSE))?
	}

	async fn on_message(&mut self, message: MessageExt, tx: &mut WsSender) -> bool {
		#[cfg(feature = "trace")]
		tracing::trace!("MessageExt({message:?})");

		match message {
			#[cfg(feature = "debug")]
			MessageExt::Message(Message::Debug(_)) => {},
			MessageExt::Message(Message::Request(c)) =>
				if !c.try_send_ws(tx, &mut self.regular_pools.requests).await {
					return false;
				},
			MessageExt::Message(Message::Batch(c)) =>
				if !c.try_send_ws(tx, &mut self.regular_pools.batches).await {
					return false;
				},
			MessageExt::Subscribe(c) =>
				if !c.try_send_ws(tx, &mut self.subscriptions).await {
					return false;
				},
			MessageExt::Unsubscribe((id, subscription_id)) => {
				let _ = self.subscriptions.remove(&id);
				let _ = self.notifications.remove(&subscription_id);
			},
		}

		true
	}

	async fn on_response(&mut self, response: WsResult<WsMessage>) -> Result<()> {
		match response {
			Ok(m) => match m {
				WsMessage::Binary(r) => {
					#[cfg(feature = "trace")]
					tracing::trace!("Response({})", String::from_utf8_lossy(&r));

					self.process_response(&r).await
				},
				WsMessage::Text(r) => {
					#[cfg(feature = "trace")]
					tracing::trace!("Response({r})");

					self.process_response(r.as_bytes()).await
				},
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

#[derive(Debug)]
enum MessageExt {
	Message(Message),
	Subscribe(Call),
	Unsubscribe((Id, SubscriptionId)),
}
struct Call {
	id: Id,
	request: String,
	notification_tx: NotificationTx,
	result_tx: ResultTx,
	subscription_tx: SubscriptionIdTx,
}
impl Call {
	async fn try_send_ws(self, tx: &mut WsSender, pool: &mut SubscriptionPool) -> bool {
		if let Err(e) = tx.send(WsMessage::Text(self.request)).await {
			try_send(self.result_tx, Err(error::Websocket::Tungstenite(e).into()), true)
		} else if try_send(self.result_tx, Ok(()), true) {
			pool.insert(self.id, (self.notification_tx, self.subscription_tx));

			true
		} else {
			false
		}
	}
}
impl Debug for Call {
	fn fmt(&self, f: &mut Formatter) -> FmtResult {
		write!(
			f,
			"Call {{ id: {}, request: {}, notification_tx: {:?}, result_tx: {:?} }}",
			self.id, self.request, self.notification_tx, self.result_tx
		)
	}
}
impl<T> CallOnce<T>
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
