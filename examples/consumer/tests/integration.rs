use near_api::{AccountId, NearGas, NearToken};
use near_sdk::serde_json::json;

// Mirror of the multi-feed's borsh `FeedUpdate` input.
#[derive(near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
struct FeedUpdate {
    feed_id: u32,
    price: u128,
    agg_ts: u64,
}

// Mirror of the `FeedData` view returned by the consumer's callbacks.
#[derive(near_sdk::serde::Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
struct FeedData {
    price: u128,
    agg_ts: u64,
    onchain_ts: u64,
}

#[tokio::test]
async fn consumer_reads_prices_from_multi_feed() -> testresult::TestResult<()> {
    // Build both contracts: the multi-feed (parent crate) and this consumer.
    let multi_feed_wasm = std::fs::read(cargo_near_build::build_with_cli(
        cargo_near_build::BuildOpts::builder()
            .manifest_path("../../Cargo.toml")
            .build(),
    )?)?;
    let consumer_wasm = std::fs::read(cargo_near_build::build_with_cli(Default::default())?)?;

    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let signer = near_api::Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY
            .parse()
            .unwrap(),
    )?;

    // Accounts.
    let owner = create_subaccount(&sandbox, "owner.sandbox").await?;
    let admin = create_subaccount(&sandbox, "admin.sandbox").await?;
    let reporter = create_subaccount(&sandbox, "reporter.sandbox").await?;
    let multi_feed = create_subaccount(&sandbox, "multi_feed.sandbox")
        .await?
        .as_contract();
    let consumer = create_subaccount(&sandbox, "consumer.sandbox")
        .await?
        .as_contract();

    // Deploy the multi-feed contract in open-read mode with `reporter` as a price reporter.
    near_api::Contract::deploy(multi_feed.account_id().clone())
        .use_code(multi_feed_wasm)
        .with_init_call(
            "new",
            json!({
                "owner": owner.account_id(),
                "open_read_enabled": true,
                "description": "Consumer Example Feed",
                "admins": [admin.account_id()],
                "products": [owner.account_id()],
                "price_reporters": [reporter.account_id()],
                "authorized_callers": [],
            }),
        )?
        .with_signer(signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Deploy the consumer, pointing it at the multi-feed contract.
    near_api::Contract::deploy(consumer.account_id().clone())
        .use_code(consumer_wasm)
        .with_init_call("new", json!({ "multi_feed": multi_feed.account_id() }))?
        .with_signer(signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Seed two feeds via the multi-feed contract.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs();
    multi_feed
        .call_function_borsh(
            "f",
            vec![
                FeedUpdate {
                    feed_id: 1,
                    price: 100,
                    agg_ts: now - 1,
                },
                FeedUpdate {
                    feed_id: 2,
                    price: 200,
                    agg_ts: now - 1,
                },
            ],
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(reporter.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Consumer reads a single price through its cross-contract call.
    let feed: Option<FeedData> = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success()
        .json()?;
    let feed = feed.unwrap();
    assert_eq!(feed.price, 100);
    assert_eq!(feed.agg_ts, now - 1);
    assert!(feed.onchain_ts > 0);

    // Consumer reads a batch, including a non-existent feed.
    let feeds: Vec<Option<FeedData>> = consumer
        .call_function(
            "get_prices",
            json!({ "feed_ids": ["0x00000001", "0x00000002", "0x00000009"] }),
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success()
        .json()?;
    assert_eq!(feeds.len(), 3);
    assert_eq!(feeds[0].as_ref().unwrap().price, 100);
    assert_eq!(feeds[0].as_ref().unwrap().agg_ts, now - 1);
    assert!(feeds[0].as_ref().unwrap().onchain_ts > 0);
    assert_eq!(feeds[1].as_ref().unwrap().price, 200);
    assert_eq!(feeds[1].as_ref().unwrap().agg_ts, now - 1);
    assert!(feeds[1].as_ref().unwrap().onchain_ts > 0);
    assert!(feeds[2].is_none());

    // Switch to gated-read mode: only whitelisted callers can read.
    multi_feed
        .call_function("set_open_read_status", json!({ "status": false }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Consumer reads are now rejected — the consumer is not whitelisted.
    let failure = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_failure();
    assert!(
        format!("{failure:?}").contains("Only authorized caller allowed"),
        "unexpected failure: {failure:?}"
    );

    // Owner (as product) adds the consumer to the authorized-caller whitelist.
    multi_feed
        .call_function(
            "set_authorized_callers",
            json!({ "updates": [{ "account": consumer.account_id(), "status": true }] }),
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Consumer can now read again.
    let feed: Option<FeedData> = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success()
        .json()?;
    assert_eq!(feed.unwrap().price, 100);

    // Admin pauses the contract; reads are blocked.
    multi_feed
        .call_function("set_paused", json!({ "paused": true }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(admin.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    let failure = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_failure();
    assert!(
        format!("{failure:?}").contains("Enforced paused"),
        "unexpected failure: {failure:?}"
    );

    // Owner switches to open-read while paused,
    // but reads are still blocked because the contract remains paused.
    multi_feed
        .call_function("set_open_read_status", json!({ "status": true }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    let failure = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_failure();
    assert!(
        format!("{failure:?}").contains("Enforced paused"),
        "unexpected failure: {failure:?}"
    );

    // Admin unpauses; reads are available again.
    multi_feed
        .call_function("set_paused", json!({ "paused": false }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(admin.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();

    let feed: Option<FeedData> = consumer
        .call_function("get_price", json!({ "feed_id": "0x00000001" }))
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success()
        .json()?;
    assert_eq!(feed.unwrap().price, 100);

    Ok(())
}

async fn create_subaccount(
    sandbox: &near_sandbox::Sandbox,
    name: &str,
) -> testresult::TestResult<near_api::Account> {
    let account_id: AccountId = name.parse().unwrap();
    sandbox
        .create_account(account_id.clone())
        .initial_balance(NearToken::from_near(10))
        .send()
        .await?;
    Ok(near_api::Account(account_id))
}
