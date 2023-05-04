//!

// crates.io
use subrpcer::net;
// subapeye
use crate::{apeye::api::prelude::*, prelude::*};

///
#[async_trait::async_trait]
pub trait Api {
	///
	async fn listening<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization;

	///
	async fn peer_count<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization;

	///
	async fn version<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization;
}
#[async_trait::async_trait]
impl<T> Api for T
where
	T: Layer,
{
	async fn listening<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(net::listening_raw()).await
	}

	async fn peer_count<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(net::peer_count_raw()).await
	}

	async fn version<R>(&self) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(net::version_raw()).await
	}
}
