//!

// crates.io
use subrpcer::state;
// subapeye
use crate::{apeye::api::prelude::*, prelude::*};

///
#[async_trait::async_trait]
pub trait Api {
	///
	async fn get_keys<P, R>(&self, prefix: P, hash: Option<&str>) -> Result<Result<R>>
	where
		P: Parameter,
		R: Deserialization;

	///
	async fn get_metadata<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization;

	///
	async fn get_pairs<P, R>(&self, prefix: P, hash: Option<&str>) -> Result<Result<R>>
	where
		P: Parameter,
		R: Deserialization;

	///
	async fn get_read_proof<K, R>(&self, keys: K, hash: Option<&str>) -> Result<Result<R>>
	where
		K: Parameter,
		R: Deserialization;

	///
	async fn get_runtime_version<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization;
}
#[async_trait::async_trait]
impl<T> Api for T
where
	T: Layer,
{
	async fn get_keys<P, R>(&self, prefix: P, hash: Option<&str>) -> Result<Result<R>>
	where
		P: Parameter,
		R: Deserialization,
	{
		self.request::<_, R>(state::get_keys_raw(prefix, hash)).await
	}

	async fn get_metadata<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(state::get_metadata_raw(hash)).await
	}

	async fn get_pairs<P, R>(&self, prefix: P, hash: Option<&str>) -> Result<Result<R>>
	where
		P: Parameter,
		R: Deserialization,
	{
		self.request::<_, R>(state::get_pairs_raw(prefix, hash)).await
	}

	async fn get_read_proof<K, R>(&self, keys: K, hash: Option<&str>) -> Result<Result<R>>
	where
		K: Parameter,
		R: Deserialization,
	{
		self.request::<_, R>(state::get_read_proof_raw(keys, hash)).await
	}

	async fn get_runtime_version<R>(&self, hash: Option<&str>) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(state::get_runtime_version_raw(hash)).await
	}
}
