use crate::provider::ApeyeProvider;

/// Subapeye
#[derive(Clone, Debug)]
pub struct Subapeye<P: ApeyeProvider> {
	provider: P,
}

impl<P: ApeyeProvider> Subapeye<P> {
	/// New subapeye instance
	pub fn new(provider: P) -> Self {
		Self { provider }
	}
}

