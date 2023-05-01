//! Full functionality WS JSONRPC client implementation.
//! Follow <https://www.jsonrpc.org/specification> specification.

pub mod initializer;
pub use initializer::*;

// std
use std::{fmt::Debug, future::Future, pin::Pin, time::Duration};
// crates.io
use futures::{
	future::{self, Either, Fuse},
	stream::{self, SplitSink, SplitStream},
	FutureExt, SinkExt, StreamExt,
};
use fxhash::FxHashMap;
use serde_json::value::RawValue;
use tokio::{
	net::TcpStream,
	sync::{mpsc, oneshot},
	time,
};
use tokio_stream::wrappers::IntervalStream;
use tokio_tungstenite::{
	tungstenite::{error::Result as WsResult, Message},
	MaybeTlsStream, WebSocketStream,
};
// subapeye
use crate::jsonrpc::{prelude::*, *};

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

type CallSender = mpsc::Sender<Call>;
type CallReceiver = mpsc::Receiver<Call>;

type WsSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type WsReceiver = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

type ErrorSender = oneshot::Sender<Error>;
type ErrorReceiver = oneshot::Receiver<Error>;

type ExitSender = oneshot::Sender<()>;
type ExitReceiver = oneshot::Receiver<()>;

type Notifier<T> = oneshot::Sender<Result<T>>;
type Pool<T> = FxHashMap<Id, T>;

type RequestResponse = JsonrpcResult;
type RequestNotifier = Notifier<RequestResponse>;
type RequestPool = Pool<RequestNotifier>;

type BatchResponse = Vec<RequestResponse>;
type BatchNotifier = Notifier<BatchResponse>;
type BatchPool = Pool<BatchNotifier>;

/// A Ws instance.
///
/// Use this to interact with the server.
#[derive(Debug)]
pub struct Ws {
	messenger: CallSender,
	request_queue: RequestQueue,
	request_timeout: Duration,
	reporter: Option<ErrorReceiver>,
	closer: Option<ExitSender>,
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
	async fn request<'a, Req>(&self, raw_request: Req) -> Result<JsonrpcResult>
	where
		Req: Send + Into<RequestRaw<'a, Value>>,
	{
		let RequestQueueGuard { lock: id, .. } = self.request_queue.consume_once()?;
		let RequestRaw { method, params } = raw_request.into();
		let (tx, rx) = oneshot::channel();

		#[cfg(feature = "debug")]
		self.messenger
			.send(Call::Debug(id))
			.await
			.map_err(|_| error::Jsonrpc::from(error::ChannelClosed::Messenger))?;
		self.messenger
			.send(Call::Single(CallInner {
				id,
				request: serde_json::to_string(&Request { jsonrpc: VERSION, id, method, params })
					.map_err(error::Generic::Serde)?,
				notifier: tx,
			}))
			.await
			.map_err(|_| error::Jsonrpc::from(error::ChannelClosed::Messenger))?;

		time::timeout(self.request_timeout, rx)
			.await
			.map_err(error::Jsonrpc::Timeout)?
			.map_err(|e| error::Jsonrpc::from(error::ChannelClosed::Notifier(e)))?
	}

	/// Send a batch of requests.
	async fn batch<'a, Req>(&self, raw_requests: Vec<Req>) -> Result<Vec<JsonrpcResult>>
	where
		Req: Send + Into<RequestRaw<'a, Value>>,
	{
		if raw_requests.is_empty() {
			Err(error::Jsonrpc::EmptyBatch)?;
		}

		let RequestQueueGuard { lock: ids, .. } = self.request_queue.consume(raw_requests.len())?;
		let id = ids
			.first()
			.ok_or(error::almost_impossible("[jsonrpc::ws] acquired `lock` is empty"))?
			.to_owned();
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

		self.messenger
			.send(Call::Batch(CallInner { id, request, notifier: tx }))
			.await
			.map_err(|_| error::Jsonrpc::from(error::ChannelClosed::Messenger))?;

		let mut responses = time::timeout(self.request_timeout, rx)
			.await
			.map_err(error::Jsonrpc::Timeout)?
			.map_err(|e| error::Jsonrpc::from(error::ChannelClosed::Notifier(e)))?;
		// Each id is unique.
		let _ = responses.as_mut().map(|r| r.sort_unstable_by_key(|r| r.id()));

		responses
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
					Either::Left((
						Either::Left((maybe_call, ws_rx_next)),
						exit_or_interval_fut_,
					)) => {
						if let Some(c) = maybe_call {
							#[cfg(feature = "trace")]
							tracing::trace!("Call({c:?})");

							if !c.try_send(&mut ws_tx, &mut pool).await {
								return;
							}
						} else {
							try_send(
								error_tx,
								error::Jsonrpc::from(error::ChannelClosed::Reporter).into(),
								false,
							);

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
						tracing::trace!("Tick(Ping)");

						if ws_tx.send(Message::Text("Ping".into())).await.is_err() {
							try_send(
								error_tx,
								error::Jsonrpc::from(error::ChannelClosed::Reporter).into(),
								false,
							);

							return;
						}

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
							try_send(error_tx, error::Jsonrpc::from(error::ChannelClosed::Reporter).into(), false);

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
						tracing::trace!("Tick(Ping)");

						if ws_tx.send(Message::Ping(Vec::new())).await.is_err() {
							try_send(error_tx, error::Jsonrpc::from(error::ChannelClosed::Reporter).into(), false);

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

#[derive(Debug)]
enum Call {
	#[cfg(feature = "debug")]
	Debug(Id),
	Single(CallInner<RequestResponse>),
	Batch(CallInner<BatchResponse>),
}
impl Call {
	async fn try_send(self, ws_tx: &mut WsSender, pool: &mut Pools) -> bool {
		match self {
			#[cfg(feature = "debug")]
			Call::Debug(_) => {},
			Call::Single(c) =>
				if !c.try_send(ws_tx, &mut pool.requests).await {
					return false;
				},
			Call::Batch(c) =>
				if !c.try_send(ws_tx, &mut pool.batches).await {
					return false;
				},
		}

		true
	}
}
// A single request object.
// `id`: Request Id.
//
// Or
//
// A batch requests object to send several request objects simultaneously.
// `id`: The first request's id.
#[derive(Debug)]
struct CallInner<T> {
	id: Id,
	request: String,
	notifier: Notifier<T>,
}
impl<T> CallInner<T>
where
	T: Debug,
{
	async fn try_send(self, ws_tx: &mut WsSender, pool: &mut Pool<Notifier<T>>) -> bool {
		if let Err(e) = ws_tx.send(Message::Text(self.request)).await {
			try_send(self.notifier, Err(e.into()), true)
		} else {
			pool.insert(self.id, self.notifier);

			true
		}
	}
}

fn try_send<T>(tx: oneshot::Sender<T>, any: T, log: bool) -> bool
where
	T: Debug,
{
	if let Err(e) = tx.send(any) {
		if log {
			tracing::error!("[jsonrpc::ws] failed to send error to outside, {e:?}");
		}

		return false;
	}

	true
}

#[derive(Debug, Default)]
struct Pools {
	requests: RequestPool,
	batches: BatchPool,
}
impl Pools {
	fn new() -> Self {
		Default::default()
	}

	async fn on_ws_recv(&mut self, response: WsResult<Message>) -> Result<()> {
		match response {
			Ok(msg) => {
				match msg {
					Message::Binary(r) => self.process_response(&r).await,
					Message::Text(r) => self.process_response(r.as_bytes()).await,
					Message::Ping(_) => tracing::trace!("ping"),
					Message::Pong(_) => tracing::trace!("pong"),
					Message::Close(_) => tracing::trace!("close"),
					Message::Frame(_) => tracing::trace!("frame"),
				}

				Ok(())
			},
			Err(e) => Err(e)?,
		}
	}

	// TODO: error handling
	async fn process_response(&mut self, response: &[u8]) {
		#[cfg(feature = "trace")]
		tracing::trace!("Response({response:?})");

		let response = response.trim_ascii_start();
		let Some(first) = response.first() else {
			tracing::error!("[jsonrpc::ws] empty response");

			return;
		};

		match first {
			b'{' =>
				if let Ok(r) = serde_json::from_slice::<ResponseResult>(response) {
					let notifier = self.requests.remove(&r.id).unwrap();

					if let Err(e) = notifier.send(Ok(Ok(r))) {
						tracing::error!("{e:?}");
					}
				} else if let Ok(e) = serde_json::from_slice::<ResponseError>(response) {
					// E.g.
					// ```
					// Response({"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":2})
					// ```

					let notifier = self.requests.remove(&e.id).unwrap();

					if let Err(e) = notifier.send(Ok(Err(e))) {
						tracing::error!("{e:?}");
					}
				},
			b'[' =>
				if let Ok(r) = serde_json::from_slice::<Vec<&RawValue>>(response) {
					let r = r
						.into_iter()
						.map(|r| {
							if let Ok(r) = serde_json::from_str::<ResponseResult>(r.get()) {
								Ok(Ok(r))
							} else if let Ok(r) = serde_json::from_str::<ResponseError>(r.get()) {
								Ok(Err(r))
							} else {
								Err(error::almost_impossible("TODO"))?
							}
						})
						.collect::<Result<Vec<JsonrpcResult>>>()
						.unwrap();

					let notifier = self.batches.remove(&r.first().unwrap().id()).unwrap();

					if let Err(e) = notifier.send(Ok(r)) {
						tracing::error!("{e:?}");
					}
				},
			_ => {
				tracing::error!("unable to process response, {response:?}");
				// TODO: return
			},
		}
	}
}
