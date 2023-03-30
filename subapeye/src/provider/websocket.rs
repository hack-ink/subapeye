use crate::provider::{ApeyeProvider, Config};

#[derive(Clone, Debug)]
pub struct WsProvider {
	config: Config,
}

impl WsProvider {
	pub fn new(config: Config) -> Self {
		Self { config }
	}
}

#[async_trait::async_trait]
impl ApeyeProvider for WsProvider {}
