// deploy.rs — Deploy and initialize the multi-feed contract on NEAR.
//
// Usage:
//   cargo run -- --client client_name
//
// Client configs are defined in ../deployments.toml.
use near_api::{Contract, Tokens};
use serde_json::json;
use std::env;
use std::io::{self, Write};

use deploy::config;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args: Vec<String> = env::args().collect();
    let client = config::parse_flag(&args, "--client")?;

    let (cfg, network_config, rpc_url, signer) = config::load_client(&client).await?;

    // Read the pre-built reproducible WASM (produced by `just build-release`).
    let wasm = std::fs::read("../target/near/multi_feed.wasm")
        .map_err(|e| eyre::eyre!("cannot read WASM — run `just build-release` first: {e}"))?;

    let account: near_api::AccountId = cfg.account.parse()?;

    // Print deployment summary and ask for confirmation.
    println!();
    println!("══════════════════════════════════════════");
    println!("  Client:          {client}");
    println!("  Account:         {account}");
    println!("  Owner:           {}", cfg.owner);
    println!("  WASM size:       {} bytes", wasm.len());
    println!("  Description:     {}", cfg.description);
    println!("  Open read:       {}", cfg.open_read_enabled);
    println!("  Admins:          {:?}", cfg.admins);
    println!("  Products:        {:?}", cfg.products);
    println!("  Price reporters: {:?}", cfg.price_reporters);
    println!("  Auth callers:    {:?}", cfg.authorized_callers);
    println!("  RPC URL:         {rpc_url}");

    // Query account balance.
    let balance = Tokens::account(account.clone())
        .near_balance()
        .fetch_from(&network_config)
        .await?;
    println!("  Account balance: {}", balance.total);

    println!("══════════════════════════════════════════");
    print!("Press Enter to deploy, Ctrl+C to abort... ");
    io::stdout().flush()?;
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;

    println!();
    println!("Deploying...");

    let result = Contract::deploy(account.clone())
        .use_code(wasm)
        .with_init_call(
            "new",
            json!({
                "owner": cfg.owner,
                "open_read_enabled": cfg.open_read_enabled,
                "description": cfg.description,
                "admins": cfg.admins,
                "products": cfg.products,
                "price_reporters": cfg.price_reporters,
                "authorized_callers": cfg.authorized_callers,
            }),
        )?
        .with_signer(signer)
        .send_to(&network_config)
        .await?;

    result.assert_success();
    println!("Deployed and initialized successfully.");

    // Verify.
    let description: String = Contract(account.clone())
        .call_function("description", json!({}))
        .read_only()
        .fetch_from(&network_config)
        .await?
        .data;
    println!("Contract description: {description}");

    Ok(())
}
