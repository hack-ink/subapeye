use array_bytes::TryFromHex;
use serde_json::Value;
use subapeye::{
	apeye::{api::*, runtime::Runtime, Apeye, Invoker},
	jsonrpc::WsInitializer,
};
use subrpcer::{chain, net};

#[tokio::main]
async fn main() {
	tracing_subscriber::fmt::init();

	enum R {}
	impl Runtime for R {
		type AccountId = [u8; 32];
		type BlockNumber = u32;
		type Hash = [u8; 32];
	}

	let apeye =
		<Apeye<_, R>>::initialize(WsInitializer::default().uri("wss://kusama-rpc.polkadot.io"))
			.await
			.unwrap();

	// let hashes = apeye.get_block_hash::<_, Vec<String>>(Some([0, 1, 2])).await.unwrap();

	// dbg!(hashes);

	// for h in hashes {
	// 	dbg!(apeye.get_block::<Value>(Some(&h)).await.unwrap());
	// 	dbg!(apeye.get_header::<Value>(Some(&h)).await.unwrap());
	// }

	// dbg!(apeye.get_finalized_head::<String>().await.unwrap());
	// dbg!(apeye.get_metadata::<String>(None).await.unwrap());

	// dbg!(apeye
	// 	.query::<String>(&apeye.query_of::<()>("System", "Number").unwrap().construct().unwrap())
	// 	.await
	// 	.unwrap());
	dbg!(apeye
		.query::<Value>(
			&apeye
				.query_of("Staking", "ErasValidatorPrefs")
				.unwrap()
				.keys(Keys::Raw(&(
					5_044_u32,
					<R as Runtime>::AccountId::try_from_hex(
						"0x305b1689cfee594c19a642a2fcd554074c93d62181c0d4117ebe196bd7c62b79"
					)
					.unwrap()
				)))
				.construct()
				.unwrap()
		)
		.await
		.unwrap()
		.unwrap());

	let v = apeye
		.batch::<_, Value>(vec![
			chain::get_block_hash_raw(<Option<()>>::None),
			chain::get_finalized_head_raw(),
			net::version_raw(),
		])
		.await
		.unwrap();
	dbg!(v);

	dbg!(apeye.version::<Value>().await.unwrap());
}
