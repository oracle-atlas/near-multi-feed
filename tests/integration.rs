use near_api::{AccountId, NearGas, NearToken};
use near_sdk::{json_types::U128, serde_json::json};

// Mirror of the contract's borsh `FeedUpdate` input (feed_id + price + agg_ts).
#[derive(near_sdk::borsh::BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
struct FeedUpdate {
    feed_id: u32,
    price: u128,
    agg_ts: u64,
}

// Mirror of the contract's JSON `FeedData` view (price + agg_ts + onchain_ts).
// price is deserialized from a decimal string to avoid IEEE-754 precision loss.
#[derive(near_sdk::serde::Deserialize, Debug)]
#[serde(crate = "near_sdk::serde")]
struct FeedData {
    price: U128,
    agg_ts: u64,
    onchain_ts: u64,
}

async fn test_basics_on(contract_wasm: Vec<u8>) -> testresult::TestResult<()> {
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let sandbox_network =
        near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);

    // Create accounts.
    let owner = create_subaccount(&sandbox, "owner.sandbox").await?;
    let reporter = create_subaccount(&sandbox, "reporter.sandbox").await?;
    let admin = create_subaccount(&sandbox, "admin.sandbox").await?;
    let outsider = create_subaccount(&sandbox, "outsider.sandbox").await?;
    let contract = create_subaccount(&sandbox, "contract.sandbox")
        .await?
        .as_contract();

    let signer = near_api::Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY
            .parse()
            .unwrap(),
    )?;

    // Deploy and initialize the contract in open-read mode.
    near_api::Contract::deploy(contract.account_id().clone())
        .use_code(contract_wasm)
        .with_init_call(
            "new",
            json!({
                "owner": owner.account_id(),
                "open_read_enabled": true,
                "description": "Integration Test Feed",
                "admins": [admin.account_id()],
                "products": [],
                "price_reporters": [reporter.account_id()],
                "authorized_callers": [],
            }),
        )?
        .with_signer(signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    // Base block time (seconds); agg_ts must be strictly newer and within drift.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs();

    // Reporter submits a batch of two feeds (borsh-encoded input).
    contract
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
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    // Read a single feed back (open-read allows any caller).
    let feed: FeedData = contract
        .call_function("fetch", json!({ "feed_id": "0x00000001" }))
        .read_only()
        .fetch_from(&sandbox_network)
        .await?
        .data;
    assert_eq!(feed.price.0, 100);
    assert_eq!(feed.agg_ts, now - 1);
    assert!(feed.onchain_ts > 0);

    // Read a batch back, including a non-existent feed id (0x00000009).
    let feeds: Vec<Option<FeedData>> = contract
        .call_function(
            "fetch_batch",
            json!({ "feed_ids": ["0x00000001", "0x00000002", "0x00000009"] }),
        )
        .read_only()
        .fetch_from(&sandbox_network)
        .await?
        .data;
    assert_eq!(feeds.len(), 3);
    assert_eq!(feeds[0].as_ref().unwrap().price.0, 100);
    assert_eq!(feeds[0].as_ref().unwrap().agg_ts, now - 1);
    assert_eq!(feeds[1].as_ref().unwrap().price.0, 200);
    assert_eq!(feeds[1].as_ref().unwrap().agg_ts, now - 1);
    assert!(feeds[2].is_none());

    // A non-reporter cannot submit feeds.
    let mut tx_execution_err = contract
        .call_function_borsh(
            "f",
            vec![FeedUpdate {
                feed_id: 3,
                price: 300,
                agg_ts: now,
            }],
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(outsider.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_failure();
    assert!(
        format!("{tx_execution_err:?}").contains("Only a price reporter can call"),
        "unexpected failure: {tx_execution_err:?}"
    );

    // An admin can pause the contract.
    contract
        .call_function("set_paused", json!({ "paused": true }))
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .gas(NearGas::from_tgas(30))
        .with_signer(admin.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    // While paused, reads are rejected.
    let rpc_querry_err = contract
        .call_function("fetch", json!({ "feed_id": "0x00000001" }))
        .read_only::<FeedData>()
        .fetch_from(&sandbox_network)
        .await
        .expect_err("reads must fail while paused");
    assert!(
        format!("{rpc_querry_err:?}").contains("Enforced paused"),
        "unexpected error: {rpc_querry_err:?}"
    );

    // A non-admin cannot unpause.
    tx_execution_err = contract
        .call_function("set_paused", json!({ "paused": false }))
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .gas(NearGas::from_tgas(30))
        .with_signer(outsider.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_failure();
    assert!(
        format!("{tx_execution_err:?}").contains("Only an admin can call"),
        "unexpected failure: {tx_execution_err:?}"
    );

    // The admin unpauses.
    contract
        .call_function("set_paused", json!({ "paused": false }))
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .gas(NearGas::from_tgas(30))
        .with_signer(admin.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    // The owner adds `outsider` as a new price reporter.
    contract
        .call_function(
            "set_price_reporters",
            json!({ "updates": [{ "account": outsider.account_id(), "status": true }] }),
        )
        .transaction()
        .deposit(NearToken::from_yoctonear(1))
        .gas(NearGas::from_tgas(30))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    // The newly added reporter can now submit feeds.
    contract
        .call_function_borsh(
            "f",
            vec![FeedUpdate {
                feed_id: 3,
                price: 300,
                agg_ts: now,
            }],
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(outsider.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_success();

    let feed: FeedData = contract
        .call_function("fetch", json!({ "feed_id": "0x00000003" }))
        .read_only()
        .fetch_from(&sandbox_network)
        .await?
        .data;
    assert_eq!(feed.price.0, 300);

    // A stale agg_ts (not strictly newer than stored) is rejected.
    tx_execution_err = contract
        .call_function_borsh(
            "f",
            vec![FeedUpdate {
                feed_id: 3,
                price: 999,
                agg_ts: now,
            }],
        )
        .transaction()
        .gas(NearGas::from_tgas(30))
        .with_signer(outsider.account_id().clone(), signer.clone())
        .send_to(&sandbox_network)
        .await?
        .assert_failure();
    assert!(
        format!("{tx_execution_err:?}").contains("Report timestamp out of bounds"),
        "unexpected failure: {tx_execution_err:?}"
    );

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

#[tokio::test]
async fn test_contract_is_operational() -> testresult::TestResult<()> {
    let contract_wasm_path = cargo_near_build::build_with_cli(Default::default())?;
    let contract_wasm = std::fs::read(contract_wasm_path)?;

    test_basics_on(contract_wasm).await
}
