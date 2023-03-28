use crate::provider::{ApeyeProvider, Config};

#[derive(Clone, Debug)]
pub struct HttpProvider {
	config: Config,
}

impl HttpProvider {
	pub fn new(config: Config) -> Self {
		Self { config }
	}
}

#[async_trait::async_trait]
impl ApeyeProvider for HttpProvider {}
