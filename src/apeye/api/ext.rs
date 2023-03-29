//!

pub use Api as ApiExt;

// crates.io
use parity_scale_codec::Encode;
use submetadatan::{Meta, StorageEntry, StorageEntryType};
use subrpcer::state;
// subapeye
use crate::{apeye::api::prelude::*, prelude::*};

///
pub trait Api: ApiQuery {}
impl<T> Api for T where T: Layer + Meta {}

///
#[async_trait::async_trait]
pub trait ApiQuery {
	///
	fn query_of<'a, E>(&'a self, pallet: &'a str, item: &'a str) -> Result<StorageQueryArgs<'a, E>>
	where
		E: EncodableArgs;

	///
	async fn query<R>(&self, storage_query: &StorageQuery) -> Result<Result<R>>
	where
		R: Deserialization;
}
#[async_trait::async_trait]
impl<T> ApiQuery for T
where
	T: Layer + Meta,
{
	fn query_of<'a, E>(&'a self, pallet: &'a str, item: &'a str) -> Result<StorageQueryArgs<'a, E>>
	where
		E: EncodableArgs,
	{
		let storage_entry = self
			.storage(pallet, item)
			.ok_or_else(|| error::Apeye::StorageNotFound(format!("{pallet}::{item}")))?;

		Ok(StorageQueryArgs::new(storage_entry))
	}

	async fn query<R>(&self, storage_query: &StorageQuery) -> Result<Result<R>>
	where
		R: Deserialization,
	{
		self.request::<_, R>(state::get_storage_raw(&storage_query.key, storage_query.at.as_ref()))
			.await
	}
}

///
pub trait EncodableArgs {
	///
	const LENGTH: usize;

	///
	fn encode(&self) -> [Vec<u8>; Self::LENGTH];
}
impl EncodableArgs for () {
	const LENGTH: usize = 0;

	fn encode(&self) -> [Vec<u8>; Self::LENGTH] {
		[]
	}
}
impl<E> EncodableArgs for (E,)
where
	E: Encode,
{
	const LENGTH: usize = 1;

	fn encode(&self) -> [Vec<u8>; Self::LENGTH] {
		[self.0.encode()]
	}
}
impl<E, E1> EncodableArgs for (E, E1)
where
	E: Encode,
	E1: Encode,
{
	const LENGTH: usize = 2;

	fn encode(&self) -> [Vec<u8>; Self::LENGTH] {
		[self.0.encode(), self.1.encode()]
	}
}
impl<E, E1, E2> EncodableArgs for (E, E1, E2)
where
	E: Encode,
	E1: Encode,
	E2: Encode,
{
	const LENGTH: usize = 3;

	fn encode(&self) -> [Vec<u8>; Self::LENGTH] {
		[self.0.encode(), self.1.encode(), self.2.encode()]
	}
}
impl<E, E1, E2, E3> EncodableArgs for (E, E1, E2, E3)
where
	E: Encode,
	E1: Encode,
	E2: Encode,
	E3: Encode,
{
	const LENGTH: usize = 4;

	fn encode(&self) -> [Vec<u8>; Self::LENGTH] {
		[self.0.encode(), self.1.encode(), self.2.encode(), self.3.encode()]
	}
}

///
pub struct StorageQuery<'a> {
	///
	pub key: String,
	///
	pub at: Option<&'a str>,
}
///
pub struct StorageQueryArgs<'a, E>
where
	E: EncodableArgs,
{
	///
	pub storage_entry: StorageEntry<'a>,
	///
	pub keys: Option<Keys<'a, E>>,
	///
	pub at: Option<&'a str>,
}
impl<'a, E> StorageQueryArgs<'a, E>
where
	E: EncodableArgs,
{
	///
	pub fn new(storage_entry: StorageEntry<'a>) -> Self {
		Self { storage_entry, keys: None, at: None }
	}

	///
	pub fn keys(mut self, keys: Keys<'a, E>) -> Self {
		self.keys = Some(keys);

		self
	}

	///
	pub fn at(mut self, at: &'a str) -> Self {
		self.at = Some(at);

		self
	}

	///
	pub fn construct(self) -> Result<StorageQuery<'a>>
	where
		[(); E::LENGTH]:,
	{
		let key = match &self.storage_entry.r#type {
			StorageEntryType::Plain =>
				substorager::storage_value_key(self.storage_entry.prefix, self.storage_entry.item),
			StorageEntryType::Map(hashers) => match self.keys.ok_or(error::Apeye::KeysNotFound)? {
				Keys::Raw(keys) => substorager::storage_n_map_key(
					self.storage_entry.prefix,
					self.storage_entry.item,
					hashers.iter().zip(keys.encode().iter()).collect::<Vec<_>>(),
				),
				Keys::Encoded(keys) => substorager::storage_n_map_key(
					self.storage_entry.prefix,
					self.storage_entry.item,
					hashers.iter().zip(keys.iter()).collect::<Vec<_>>(),
				),
			},
		}
		.to_string();

		#[cfg(feature = "trace")]
		tracing::trace!("StorageKey({key})");

		Ok(StorageQuery { key, at: self.at })
	}
}

///
pub enum Keys<'a, E>
where
	E: EncodableArgs,
{
	///
	Raw(&'a E),
	///
	Encoded(&'a [&'a [u8]]),
	// ///
	// Hashed(&'a [&'a [u8]]),
}
