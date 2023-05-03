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
	jsonrpc::{Connection, Initialize, IntoRequestRaw, ResponseResult, ResultExt, Subscriber},
	prelude::*,
};

///
pub trait Layer: Invoker + Runtime {}
impl<T> Layer for T where T: Invoker + Runtime {}

///
#[async_trait::async_trait]
pub trait Invoker: Send + Sync {
	///
	type Connection: Connection;

	///
	fn map_result<R>(r: ResponseResult) -> Result<R>
	where
		R: DeserializeOwned,
	{
		Ok(serde_json::from_value(r.extract_err()?.result).map_err(error::Generic::Serde)?)
	}

	///
	async fn request<'a, Req, Resp>(&self, raw_request: Req) -> Result<Result<Resp>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned;

	///
	async fn batch<'a, Req, Resp>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned;

	///
	async fn subscribe<'a, Req, M, Resp>(
		&self,
		raw_request: Req,
		unsubscribe_method: M,
	) -> Result<Subscriber<'a, Self::Connection, Req, Resp>>
	where
		Req: IntoRequestRaw<'a>,
		M: Send + AsRef<str>,
		Resp: DeserializeOwned;
}
#[async_trait::async_trait]
impl<T> Invoker for T
where
	T: Connection,
{
	type Connection = T;

	async fn request<'a, Req, R>(&self, raw_request: Req) -> Result<Result<R>>
	where
		Req: IntoRequestRaw<'a>,
		R: DeserializeOwned,
	{
		Ok(self.request(raw_request).await.map(Self::map_result)?)
	}

	async fn batch<'a, Req, Resp>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned,
	{
		Ok(self.batch(raw_requests).await?.into_iter().map(Self::map_result).collect())
	}

	async fn subscribe<'a, Req, M, Resp>(
		&self,
		raw_request: Req,
		unsubscribe_method: M,
	) -> Result<Subscriber<'a, Self::Connection, Req, Resp>>
	where
		Req: IntoRequestRaw<'a>,
		M: Send + AsRef<str>,
		Resp: DeserializeOwned,
	{
		Ok(self.subscribe(raw_request, unsubscribe_method).await.unwrap())
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
				.expect("[apeye] failed to parse metadata");

		// #[cfg(feature = "trace")]
		// tracing::trace!("Metadata({:?})", apeye.metadata);

		Ok(apeye)
	}
}
#[async_trait::async_trait]
impl<I, R> Invoker for Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	type Connection = I::Connection;

	async fn request<'a, Req, Resp>(&self, raw_request: Req) -> Result<Result<Resp>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned,
	{
		self.invoker.request(raw_request).await
	}

	async fn batch<'a, Req, Resp>(&self, raw_requests: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<'a>,
		Resp: DeserializeOwned,
	{
		self.invoker.batch(raw_requests).await
	}

	async fn subscribe<'a, Req, M, Resp>(
		&self,
		raw_request: Req,
		unsubscribe_method: M,
	) -> Result<Subscriber<'a, Self::Connection, Req, Resp>>
	where
		Req: IntoRequestRaw<'a>,
		M: Send + AsRef<str>,
		Resp: DeserializeOwned,
	{
		self.invoker.subscribe(raw_request, unsubscribe_method).await
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
