//!

// crates.io
use array_bytes::{Hex, TryFromHex};

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
