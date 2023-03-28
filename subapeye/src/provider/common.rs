
/// provider config
#[derive(Clone, Debug)]
pub struct Config {
	pub endpoint: String,
	pub timeout: Option<u64>,
}

impl Config {
	pub fn simple(endpoint: impl AsRef<str>) -> Self {
		Self {
			endpoint: endpoint.as_ref().to_string(),
			timeout: None,
		}
	}
}

#[async_trait::async_trait]
pub trait ApeyeProvider: Clone + std::fmt::Debug {

}


