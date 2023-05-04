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
	jsonrpc::*,
	prelude::{error, Result},
};

///
pub trait Layer: Invoker + Runtime {}
impl<T> Layer for T where T: Invoker + Runtime {}

///
pub trait LayerExt: Layer + InvokerExt {}
impl<T> LayerExt for T where T: Layer + InvokerExt {}

///
#[async_trait::async_trait]
pub trait Invoker: Send + Sync {
	///
	fn map_result<R>(r: ResponseResult) -> Result<R>
	where
		R: DeserializeOwned,
	{
		Ok(serde_json::from_value(r.extract_err()?.result).map_err(error::Generic::Serde)?)
	}

	///
	async fn request<'a, Req, Resp>(&self, request_raw: Req) -> Result<Result<Resp>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned;

	///
	async fn batch<'a, Req, Resp>(&self, requests_raw: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned;
}
#[async_trait::async_trait]
impl<T> Invoker for T
where
	T: Jsonrpc,
{
	async fn request<'a, Req, R>(&self, request_raw: Req) -> Result<Result<R>>
	where
		Req: IntoRequestRaw<&'a str>,
		R: DeserializeOwned,
	{
		let r = request_raw.into();

		Ok(self
			.request(RequestRaw { method: r.method.into(), params: r.params })
			.await
			.map(Self::map_result)?)
	}

	async fn batch<'a, Req, Resp>(&self, requests_raw: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned,
	{
		Ok(self.batch(requests_raw).await?.into_iter().map(Self::map_result).collect())
	}
}
///
#[async_trait::async_trait]
pub trait InvokerExt: Invoker {
	///
	async fn subscribe<'a, Req, Resp>(
		&self,
		request_raw: Req,
		unsubscribe_method: String,
	) -> Result<Subscriber<Resp>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned;
}
#[async_trait::async_trait]
impl<T> InvokerExt for T
where
	T: Invoker + JsonrpcExt,
{
	async fn subscribe<'a, Req, Resp>(
		&self,
		request_raw: Req,
		unsubscribe_method: String,
	) -> Result<Subscriber<Resp>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned,
	{
		Ok(self.subscribe(request_raw, unsubscribe_method).await.unwrap())
	}
}

/// The API client for Substrate-like chain.
#[derive(Clone, Debug)]
pub struct Apeye<I, R> {
	///
	pub invoker: Arc<I>,
	///
	pub metadata: Metadata,
	///
	pub runtime: PhantomData<R>,
}
impl<Ivk, R> Apeye<Ivk, R>
where
	Ivk: Invoker,
	R: Runtime,
{
	/// Initialize the API client with the given initializer.
	pub async fn initialize<Iz>(initializer: Iz) -> Result<Self>
	where
		Iz: Initialize<Protocol = Ivk>,
	{
		let invoker = Arc::new(initializer.initialize().await?);
		let mut apeye = Self { invoker, metadata: Default::default(), runtime: Default::default() };

		apeye.metadata =
			submetadatan::unprefix_raw_metadata_minimal(apeye.get_metadata::<String>(None).await??)
				.expect("[apeye] failed to parse metadata");

		Ok(apeye)
	}
}
#[async_trait::async_trait]
impl<I, Rt> Invoker for Apeye<I, Rt>
where
	I: Invoker,
	Rt: Runtime,
{
	async fn request<'a, Req, Resp>(&self, request_raw: Req) -> Result<Result<Resp>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned,
	{
		self.invoker.request(request_raw).await
	}

	async fn batch<'a, Req, Resp>(&self, requests_raw: Vec<Req>) -> Result<Vec<Result<Resp>>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned,
	{
		self.invoker.batch(requests_raw).await
	}
}
#[async_trait::async_trait]
impl<I, Rt> InvokerExt for Apeye<I, Rt>
where
	I: InvokerExt,
	Rt: Runtime,
{
	async fn subscribe<'a, Req, Resp>(
		&self,
		request_raw: Req,
		unsubscribe_method: String,
	) -> Result<Subscriber<Resp>>
	where
		Req: IntoRequestRaw<&'a str>,
		Resp: DeserializeOwned,
	{
		self.invoker.subscribe(request_raw, unsubscribe_method).await
	}
}
impl<I, R> Runtime for Apeye<I, R>
where
	I: Send + Sync,
	R: Runtime,
{
	type AccountId = R::AccountId;
	type BlockNumber = R::BlockNumber;
	type Hash = R::Hash;
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
