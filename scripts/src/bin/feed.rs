// feed.rs — Submit price feed data to the multi-feed contract.
//
// Usage:
//   cargo run --bin feed -- --account multi-feed-qa-1.atlas-oracle.near --count 5
//
// Secrets (private key, RPC URL) are in ../.env.
use borsh::BorshSerialize;
use deploy::config;
use near_api::{Contract, NearGas};
use std::env;

#[derive(BorshSerialize)]
struct FeedUpdate {
    feed_id: u32,
    price: u128,
    agg_ts: u64,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args: Vec<String> = env::args().collect();
    let account = config::parse_flag(&args, "--account")?;
    let count: u32 = config::parse_flag(&args, "--count")?.parse()?;

    let (network_config, reporter_account, signer) = config::load_feed_client().await?;

    let account: near_api::AccountId = account.parse()?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)?
        .as_secs();

    let updates: Vec<FeedUpdate> = (1..=count)
        .map(|feed_id| FeedUpdate {
            feed_id,
            price: (100 * feed_id) as u128,
            agg_ts: now - 1,
        })
        .collect();

    println!("Feeding {account} with {} updates...", updates.len());

    Contract(account.clone())
        .call_function_borsh("f", updates)
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(reporter_account.clone(), signer)
        .send_to(&network_config)
        .await?
        .assert_success();

    println!("Done.");
    Ok(())
}
