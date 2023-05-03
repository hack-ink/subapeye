use std::error::Error;

use array_bytes::TryFromHex;
use serde_json::Value;
use subapeye::{
	apeye::{api::*, runtime::Runtime, Apeye, Invoker},
	jsonrpc::WsInitializer,
};
use subrpcer::{chain, net};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
	tracing_subscriber::fmt::init();

	enum R {}
	impl Runtime for R {
		type AccountId = [u8; 32];
		type BlockNumber = u32;
		type Hash = [u8; 32];
	}

	let apeye =
		<Apeye<_, R>>::initialize(WsInitializer::default().uri("wss://kusama-rpc.polkadot.io"))
			.await?;

	// for h in apeye.get_block_hash::<_, Vec<String>>(Some([0, 1, 2])).await? {
	// 	dbg!(apeye.get_block::<Value>(Some(&h)).await?);
	// 	dbg!(apeye.get_header::<Value>(Some(&h)).await?);
	// }

	// dbg!(apeye.get_finalized_head::<String>().await?);
	// dbg!(apeye.query::<String>(&apeye.query_of::<()>("System", "Number")?.construct()?).await?);
	// dbg!(
	// 	apeye
	// 		.query::<Value>(
	// 			&apeye
	// 				.query_of("Staking", "ErasValidatorPrefs")?
	// 				.keys(Keys::Raw(&(
	// 					5_044_u32,
	// 					<R as Runtime>::AccountId::try_from_hex(
	// 						"0x305b1689cfee594c19a642a2fcd554074c93d62181c0d4117ebe196bd7c62b79"
	// 					)
	// 					.unwrap()
	// 				)))
	// 				.construct()?
	// 		)
	// 		.await?
	// );
	// dbg!(
	// 	apeye
	// 		.batch::<_, Value>(vec![
	// 			chain::get_block_hash_raw(<Option<()>>::None),
	// 			chain::get_finalized_head_raw(),
	// 			net::version_raw(),
	// 		])
	// 		.await?
	// );
	dbg!(apeye.version::<Value>().await?);

	apeye
		.subscribe::<_, _, ()>(
			(
				"state_subscribeStorage",
				serde_json::json!([[
					"0x26aa394eea5630e07c48ae0c9558cef70a98fdbe9ce6c55837576c60c7af3850"
				]]),
			),
			"state_unsubscribeStorage",
		)
		.await
		.unwrap();

	Ok(())
}
