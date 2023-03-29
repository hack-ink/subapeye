//!

pub mod api;
use api::ApiState;

pub mod runtime;
use runtime::Runtime;

// std
use std::{marker::PhantomData, sync::Arc};
// crates.io
use serde::de::DeserializeOwned;
use submetadatan::{Meta, Metadata, StorageEntry};
// subapeye
use crate::{
	jsonrpc::{Connection, Initialize, IntoRequestRaw, JsonrpcResult},
	prelude::*,
};

///
pub trait Layer: Invoker + Runtime {}
impl<T> Layer for T where T: Invoker + Runtime {}

///
#[async_trait::async_trait]
pub trait Invoker: Send + Sync {
	///
	fn map_result<R>(r: JsonrpcResult) -> Result<R>
	where
		R: DeserializeOwned,
	{
		Ok(r.map_err(|e| error::Jsonrpc::ResponseError(e.error))?)
			.and_then(|r| Ok(serde_json::from_value::<R>(r.result).map_err(error::Generic::Serde)?))
	}

	///
	async fn request<'a, Req, R>(&self, raw_request: Req) -> Result<Result<R>>
	where
		Req: IntoRequestRaw<'a>,
		R: DeserializeOwned;

	///
	async fn batch<'a, Req, R>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<R>>>
	where
		Req: IntoRequestRaw<'a>,
		R: DeserializeOwned;
}
#[async_trait::async_trait]
impl<T> Invoker for T
where
	T: Connection,
{
	async fn request<'a, Req, R>(&self, raw_request: Req) -> Result<Result<R>>
	where
		Req: IntoRequestRaw<'a>,
		R: DeserializeOwned,
	{
		self.request(raw_request).await.map(Self::map_result)
	}

	async fn batch<'a, Req, R>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<R>>>
	where
		Req: IntoRequestRaw<'a>,
		R: DeserializeOwned,
	{
		self.batch(raw_requests).await.map(|v| v.into_iter().map(Self::map_result).collect())
	}
}

/// The API client for Substrate-like chain.
#[derive(Clone, Debug)]
pub struct Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	///
	pub invoker: Arc<I>,
	///
	pub metadata: Metadata,
	///
	pub runtime: PhantomData<R>,
}
impl<I, R> Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	/// Initialize the API client with the given initializer.
	pub async fn initialize<Iz>(initializer: Iz) -> Result<Self>
	where
		Iz: Initialize<Connection = I>,
	{
		let invoker = Arc::new(initializer.initialize().await?);
		let mut apeye = Self { invoker, metadata: Default::default(), runtime: Default::default() };

		apeye.metadata =
			submetadatan::unprefix_raw_metadata_minimal(apeye.get_metadata::<String>(None).await??)
				.map_err(error::Generic::Submetadatan)?;

		#[cfg(feature = "trace")]
		tracing::trace!("Metadata({:?})", apeye.metadata);

		Ok(apeye)
	}
}
#[async_trait::async_trait]
impl<I, R> Invoker for Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	async fn request<'a, Req, Resp>(&self, raw_request: Req) -> Result<Result<Resp>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned,
	{
		I::request::<_, _>(&self.invoker, raw_request).await
	}

	async fn batch<'a, Req, Resp>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned,
	{
		I::batch::<_, _>(&self.invoker, raw_requests).await
	}
}
impl<I, R> Meta for Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	fn storage<'a, 'b>(&'a self, pallet: &str, item: &'b str) -> Option<StorageEntry<'b>>
	where
		'a: 'b,
	{
		self.metadata.storage(pallet, item)
	}
}
