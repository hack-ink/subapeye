//! JSONRPC client library.

pub mod ws;
pub use ws::{Ws, WsInitializer};

pub mod error;
pub use error::Error;

pub mod prelude {
	//! JSONRPC prelude.

	pub use std::result::Result as StdResult;

	pub use crate::jsonrpc::error::{self, Error};

	/// Subapeye's `Result` type.
	pub type Result<T> = StdResult<T, Error>;
}
use prelude::*;

// std
use std::{
	fmt::{Debug, Formatter, Result as FmtResult},
	future::Future,
	hash::Hash,
	marker::PhantomData,
	pin::Pin,
	sync::{
		atomic::{AtomicUsize, Ordering},
		Arc,
	},
};
// crates.io
use fxhash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::{value::RawValue, Value};
use tokio::sync::{mpsc, oneshot};

/// JSONRPC Id.
pub type Id = usize;
/// Subscription Id.
pub type SubscriptionId = String;

///
pub type ResponseResult = StdResult<ResponseOk, JsonrpcError>;
///
pub type SubscriptionResult = StdResult<SubscriptionOk, JsonrpcError>;

type MessageTx = mpsc::Sender<Message>;
type MessageRx = mpsc::Receiver<Message>;

type ErrorTx = oneshot::Sender<Error>;
type ErrorRx = oneshot::Receiver<Error>;

type ExitTx = oneshot::Sender<()>;
type ExitRx = oneshot::Receiver<()>;

type ResponseTx<T> = oneshot::Sender<Result<T>>;

type SubscriptionTx = mpsc::Sender<SubscriptionResult>;
type SubscriptionRx = mpsc::Receiver<SubscriptionResult>;

type RequestTx = ResponseTx<ResponseResult>;
type BatchTx = ResponseTx<Vec<ResponseResult>>;

type Pool<K, V> = FxHashMap<K, V>;
type RequestPool = Pool<Id, RequestTx>;
type BatchPool = Pool<Id, BatchTx>;
type SubscriptionPool = Pool<SubscriptionId, SubscriptionTx>;

/// JSONRPC version.
pub const VERSION: &str = "2.0";

const E_EMPTY_LOCK: &str = "[jsonrpc] acquired `lock` is empty";
const E_INVALID_RESPONSE: &str = "[jsonrpc] unable to process response";
const E_MESSAGE_CHANNEL_CLOSED: &str = "[jsonrpc] message channel closed";
const E_NO_ERROR: &str = "[jsonrpc] no error to report";
const E_REPORTER_CHANNEL_CLOSED: &str = "[jsonrpc] reporter channel closed";
const E_TX_NOT_FOUND: &str = "[jsonrpc] tx not found in the pool";

///
#[async_trait::async_trait]
pub trait Initialize {
	///
	type Connection;

	///
	async fn initialize(self) -> Result<Self::Connection>;
}
#[async_trait::async_trait]
impl<'a> Initialize for WsInitializer<'a> {
	type Connection = Ws;

	async fn initialize(self) -> Result<Self::Connection> {
		self.initialize().await
	}
}

///
pub trait IntoRequestRaw<'a>: Send + Into<RequestRaw<'a, Value>> {}
impl<'a, T> IntoRequestRaw<'a> for T where T: Send + Into<RequestRaw<'a, Value>> {}

///
#[async_trait::async_trait]
pub trait Jsonrpc: Sized {
	/// Send a single request.
	async fn request<'a, R>(&self, raw_request: R) -> Result<ResponseResult>
	where
		R: IntoRequestRaw<'a>;

	/// Send a batch of requests.
	async fn batch<'a, R>(&self, raw_requests: Vec<R>) -> Result<Vec<ResponseResult>>
	where
		R: IntoRequestRaw<'a>;

	/// Send a subscription.
	async fn subscribe<'a, R, M, D>(
		&self,
		raw_request: R,
		unsubscribe_method: M,
	) -> Result<Subscriber<'a, Self, R, D>>
	where
		R: IntoRequestRaw<'a>,
		M: Send + AsRef<str>;
}

///
pub trait Connection: Send + Sync + Jsonrpc {}
impl<T> Connection for T where T: Send + Sync + Jsonrpc {}

///
pub trait ResultExt {
	///
	type Ok;

	///
	fn id(&self) -> Id;

	///
	fn extract_err(self) -> Result<Self::Ok>;
}
impl ResultExt for ResponseResult {
	type Ok = ResponseOk;

	fn id(&self) -> Id {
		match self {
			Self::Ok(o) => o.id,
			Self::Err(e) => e.id,
		}
	}

	fn extract_err(self) -> Result<<Self as ResultExt>::Ok> {
		Ok(self.map_err(|e| error::Jsonrpc::Response(e.error))?)
	}
}

trait PoolExt {
	type Key: PartialEq + Eq + Hash;
	type Value;

	fn take_tx(&mut self, key: &Self::Key) -> Self::Value;
}
impl<K, V> PoolExt for Pool<K, V>
where
	K: PartialEq + Eq + Hash,
{
	type Key = K;
	type Value = V;

	fn take_tx(&mut self, key: &Self::Key) -> Self::Value {
		self.remove(key).expect(E_TX_NOT_FOUND)
	}
}

/// Generic JSONRPC request.
#[allow(missing_docs)]
#[derive(Clone, Debug, Serialize)]
pub struct Request<'a, P> {
	#[serde(borrow)]
	pub jsonrpc: &'a str,
	pub id: Id,
	#[serde(borrow)]
	pub method: &'a str,
	pub params: P,
}
/// Raw JSONRPC request.
#[allow(missing_docs)]
#[derive(Clone, Debug)]
pub struct RequestRaw<'a, P> {
	pub method: &'a str,
	pub params: P,
}
impl<'a, P> From<(&'a str, P)> for RequestRaw<'a, P> {
	fn from(raw: (&'a str, P)) -> Self {
		Self { method: raw.0, params: raw.1 }
	}
}

/// Generic JSONRPC response result.
#[allow(missing_docs)]
#[derive(Clone, Debug, Deserialize)]
pub struct ResponseOk {
	pub jsonrpc: String,
	pub id: Id,
	pub result: Value,
}

/// Generic JSONRPC error.
#[allow(missing_docs)]
#[derive(Clone, Debug, Deserialize)]
pub struct JsonrpcError {
	pub jsonrpc: String,
	pub id: Id,
	pub error: Value,
}

/// Generic JSONRPC notification.
#[allow(missing_docs)]
#[derive(Clone, Debug, Deserialize)]
pub struct SubscriptionOk {
	pub jsonrpc: String,
	pub method: String,
	pub params: Value,
}

///
#[derive(Debug)]
pub struct Subscriber<'a, J, R, D>
where
	J: Jsonrpc,
	R: IntoRequestRaw<'a>,
{
	unsubscribe_fut: UnsubscribeFut<'a, J, R>,
	subscription_rx: SubscriptionRx,
	_deserialize: D,
}
struct UnsubscribeFut<'a, J, R>
where
	J: Jsonrpc,
	R: IntoRequestRaw<'a>,
{
	f: Box<dyn FnOnce(&J, R) -> Pin<Box<dyn Future<Output = ()>>>>,
	_lifetime: PhantomData<&'a ()>,
}
impl<'a, J, R> Debug for UnsubscribeFut<'a, J, R>
where
	J: Jsonrpc,
	R: IntoRequestRaw<'a>,
{
	fn fmt(&self, f: &mut Formatter) -> FmtResult {
		write!(f, "UnsubscribeFut")
	}
}

#[derive(Debug)]
struct RequestQueue {
	size: Id,
	active: Arc<()>,
	next: AtomicUsize,
}
impl RequestQueue {
	fn with_size(size: Id) -> Self {
		Self { size, active: Default::default(), next: Default::default() }
	}

	fn consume_once(&self) -> Result<RequestQueueGuard<Id>> {
		let active = Arc::strong_count(&self.active);

		#[cfg(feature = "trace")]
		tracing::trace!("RequestQueue({active}/{})", self.size);

		if active == self.size {
			Err(error::Jsonrpc::ExceededRequestQueueMaxSize(self.size))?
		} else {
			Ok(RequestQueueGuard {
				lock: self.next.fetch_add(1, Ordering::SeqCst),
				_strong: self.active.clone(),
			})
		}
	}

	fn consume(&self, count: Id) -> Result<RequestQueueGuard<Vec<Id>>> {
		let active = Arc::strong_count(&self.active);

		#[cfg(feature = "trace")]
		tracing::trace!("RequestQueue({active}/{})", self.size);

		if active == self.size {
			Err(error::Jsonrpc::ExceededRequestQueueMaxSize(self.size))?
		} else {
			Ok(RequestQueueGuard {
				lock: (0..count).map(|_| self.next.fetch_add(1, Ordering::SeqCst)).collect(),
				_strong: self.active.clone(),
			})
		}
	}
}

#[derive(Debug)]
struct RequestQueueGuard<L> {
	lock: L,
	_strong: Arc<()>,
}

#[derive(Debug)]
enum Message {
	#[cfg(feature = "debug")]
	Debug(Id),
	Request(Call<ResponseResult>),
	Batch(Call<Vec<ResponseResult>>),
	Subscription(Subscription),
}
// A single request object.
// `id`: Request Id.
//
// Or
//
// A batch requests object to send several request objects simultaneously.
// `id`: The first request's id.
struct Call<T> {
	id: Id,
	request: String,
	tx: ResponseTx<T>,
}
impl<T> Debug for Call<T>
where
	T: Debug,
{
	fn fmt(&self, f: &mut Formatter) -> FmtResult {
		write!(f, "Call {{ id: {}, request: {}, tx: {:?} }}", self.id, self.request, self.tx)
	}
}
#[derive(Debug)]
struct Subscription {
	id: String,
	tx: SubscriptionTx,
}

#[derive(Debug, Default)]
struct Pools {
	requests: RequestPool,
	batches: BatchPool,
	subscriptions: SubscriptionPool,
}
impl Pools {
	fn new() -> Self {
		Default::default()
	}

	async fn process_response(&mut self, response: &[u8]) -> Result<()> {
		let r = response.trim_ascii_start();
		let first = r.first().ok_or(error::Jsonrpc::EmptyResponse)?;

		match first {
			b'{' =>
				if let Ok(o) = serde_json::from_slice::<ResponseOk>(r) {
					self.requests.take_tx(&o.id).send(Ok(Ok(o))).expect(E_MESSAGE_CHANNEL_CLOSED);

					return Ok(());
				} else if let Ok(e) = serde_json::from_slice::<JsonrpcError>(r) {
					// E.g.
					// ```
					// Response({"jsonrpc":"2.0","error":{"code":-32601,"message":"Method not found"},"id":2})
					// ```

					self.requests.take_tx(&e.id).send(Ok(Err(e))).expect(E_MESSAGE_CHANNEL_CLOSED);

					return Ok(());
				} else if let Ok(o) = serde_json::from_slice::<SubscriptionOk>(r) {
					dbg!(o);

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

					self.batches
						.take_tx(&r.first().ok_or(error::Jsonrpc::EmptyBatch)?.id())
						.send(Ok(r))
						.expect(E_MESSAGE_CHANNEL_CLOSED);

					return Ok(());
				},
			_ => (),
		}

		Err(error::almost_impossible(E_INVALID_RESPONSE))?
	}
}

fn try_send<T>(tx: oneshot::Sender<T>, any: T, log: bool) -> bool
where
	T: Debug,
{
	if let Err(e) = tx.send(any) {
		if log {
			tracing::error!("[jsonrpc] failed to throw this error to outside, {e:?}");
		}

		return false;
	}

	true
}
