//!

// crates.io
use subrpcer::chain;
// subapeye
use crate::{apeye::api::prelude::*, prelude::*};

///
#[async_trait::async_trait]
pub trait Api {
	///
	async fn get_block<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization;

	///
	async fn get_block_hash<LoV, R>(&self, list_or_value: Option<LoV>) -> Result<Result<R>>
	where
		LoV: Parameter,
		R: Deserialization;

	///
	async fn get_finalized_head<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization;

	///
	async fn get_header<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization;
}
#[async_trait::async_trait]
impl<T> Api for T
where
	T: Layer,
{
	async fn get_block<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(chain::get_block_raw(hash)).await
	}

	async fn get_block_hash<LoV, R>(&self, list_or_value: Option<LoV>) -> Result<Result<R>>
	where
		LoV: Parameter,
		R: Deserialization,
	{
		self.request::<_, R>(chain::get_block_hash_raw(list_or_value)).await
	}

	async fn get_finalized_head<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(chain::get_finalized_head_raw()).await
	}

	///
	async fn get_header<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(chain::get_header_raw(hash)).await
	}
}
