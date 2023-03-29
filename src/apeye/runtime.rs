//!

// crates.io
use array_bytes::{Hex, TryFromHex};
// subapeye
use crate::apeye::{Apeye, Invoker};

///
pub trait Runtime: Send + Sync {
	///
	type BlockNumber: ParameterConvertor;
	///
	type Hash: ParameterConvertor;
	///
	type AccountId: ParameterConvertor;
}

///
pub trait ParameterConvertor: Hex + TryFromHex {}
impl<T> ParameterConvertor for T where T: Hex + TryFromHex {}

impl<I, R> Runtime for Apeye<I, R>
where
	I: Invoker,
	R: Runtime,
{
	type AccountId = R::AccountId;
	type BlockNumber = R::BlockNumber;
	type Hash = R::Hash;
}
