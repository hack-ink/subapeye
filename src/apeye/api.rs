//!

pub mod prelude {
	//!

	pub use crate::apeye::{
		api::{Argument, Deserialization, Parameter},
		runtime::Runtime,
		Layer, LayerExt,
	};
}

pub mod base;
pub use base::*;

pub mod ext;
pub use ext::*;

pub mod frontier;
pub use frontier::*;

// std
use std::fmt::Debug;
// crates.io
use serde::{de::DeserializeOwned, Serialize};

///
pub trait Api: ApiBase + ApiExt + ApiFrontier {}
impl<T> Api for T where T: ApiBase + ApiExt + ApiFrontier {}

///
pub trait Argument: Debug + Send + Sync {}
impl<T> Argument for T where T: Debug + Send + Sync {}

///
pub trait Parameter: Serialize + Argument {}
impl<T> Parameter for T where T: Serialize + Argument {}

///
pub trait Deserialization: Debug + DeserializeOwned {}
impl<T> Deserialization for T where T: Debug + DeserializeOwned {}
