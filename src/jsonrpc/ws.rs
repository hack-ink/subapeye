//! Full functionality WS JSONRPC client implementation.
//! Follow <https://www.jsonrpc.org/specification> specification.

pub mod initializer;
pub use initializer::*;

// std
use std::{fmt::Debug, time::Duration};
// crates.io
use futures::{
	stream::{SplitSink, SplitStream},
	SinkExt,
};
use fxhash::FxHashMap;
use serde_json::value::RawValue;
use tokio::{
	net::TcpStream,
	sync::{mpsc, oneshot},
	time,
};
use tokio_tungstenite::{
	tungstenite::{error::Result as WsResult, Message},
	MaybeTlsStream, WebSocketStream,
};
// subapeye
use crate::{
	jsonrpc::*,
	prelude::{Error, *},
};

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
		self.messenger.send(Call::Debug(id)).await.map_err(|_| error::Tokio::ChannelClosed)?;
		self.messenger
			.send(Call::Single(CallInner {
				id,
				request: serde_json::to_string(&Request { jsonrpc: VERSION, id, method, params })
					.map_err(error::Generic::Serde)?,
				notifier: tx,
			}))
			.await
			.map_err(|_| error::Tokio::ChannelClosed)?;

		time::timeout(self.request_timeout, rx)
			.await
			.map_err(error::Tokio::Elapsed)?
			.map_err(error::Tokio::OneshotRecv)?
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
			.map_err(|_| error::Tokio::ChannelClosed)?;

		let mut responses = time::timeout(self.request_timeout, rx)
			.await
			.map_err(error::Tokio::Elapsed)?
			.map_err(error::Tokio::OneshotRecv)?;
		// Each id is unique.
		let _ = responses.as_mut().map(|r| r.sort_unstable_by_key(|r| r.id()));

		responses
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
			try_send(self.notifier, Err(e.into()))
		} else {
			pool.insert(self.id, self.notifier);

			true
		}
	}
}

fn try_send<T>(tx: oneshot::Sender<T>, any: T) -> bool
where
	T: Debug,
{
	if let Err(e) = tx.send(any) {
		tracing::error!("[jsonrpc::ws] failed to send error to outside, {e:?}");

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
					if r.id == 0 {
						return;
					}

					let notifier = self.requests.remove(&r.id).unwrap();

					if let Err(e) = notifier.send(Ok(Ok(r))) {
						tracing::error!("{e:?}");
					}
				} else if let Ok(e) = serde_json::from_slice::<ResponseError>(response) {
					dbg!(e);
					// Response({"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not
					// found"},"id":2})
					// TODO: return
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
