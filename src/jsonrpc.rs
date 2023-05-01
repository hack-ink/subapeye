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
use std::sync::{
	atomic::{AtomicUsize, Ordering},
	Arc,
};
// crates.io
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSONRPC Id.
pub type Id = usize;

///
pub type JsonrpcResult = StdResult<ResponseResult, ResponseError>;

/// JSONRPC version.
pub const VERSION: &str = "2.0";

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
		// #[cfg(feature = "trace")]
		// tracing::trace!("Connecting({uri})");

		self.initialize().await
	}
}

///
pub trait IntoRequestRaw<'a>: Send + Into<RequestRaw<'a, Value>> {}
impl<'a, T> IntoRequestRaw<'a> for T where T: Send + Into<RequestRaw<'a, Value>> {}

///
#[async_trait::async_trait]
pub trait Jsonrpc {
	/// Send a single request.
	async fn request<'a, Req>(&self, raw_request: Req) -> Result<JsonrpcResult>
	where
		Req: IntoRequestRaw<'a>;

	/// Send a single request.
	async fn batch<'a, Req>(&self, raw_requests: Vec<Req>) -> Result<Vec<JsonrpcResult>>
	where
		Req: IntoRequestRaw<'a>;
}

///
pub trait Connection: Send + Sync + Jsonrpc {}
impl<T> Connection for T where T: Send + Sync + Jsonrpc {}

///
pub trait Response {
	///
	fn id(&self) -> Id;

	// ///
	// fn deserialize(self) -> Self;
}
impl Response for JsonrpcResult {
	fn id(&self) -> Id {
		match self {
			Self::Ok(r) => r.id,
			Self::Err(e) => e.id,
		}
	}

	// fn deserialize(self) -> Result<Self> {
	// 	Ok(match self {
	// 		Self::Ok(r) => Self::Ok(ResponseResult {
	// 			jsonrpc: r.jsonrpc,
	// 			id: r.id,
	// 			result: serde_json::to_value(r.result).map_err(error::Generic::Serde)?,
	// 		}),
	// 		Self::Err(e) => Self::Err(Response {
	// 			jsonrpc: e.jsonrpc,
	// 			id: e.id,
	// 			error: serde_json::to_value(e.error).map_err(error::Generic::Serde)?,
	// 		}),
	// 	})
	// }
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

/// Generic JSONRPC result.
#[allow(missing_docs)]
#[derive(Clone, Debug, Deserialize)]
pub struct ResponseResult {
	pub jsonrpc: String,
	pub id: Id,
	pub result: Value,
}

/// Generic JSONRPC error.
#[allow(missing_docs)]
#[derive(Clone, Debug, Deserialize)]
pub struct ResponseError {
	pub jsonrpc: String,
	pub id: Id,
	pub error: Value,
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
