// config.rs — Shared project config loading for deploy and feed scripts.
//
// Reads client config from ../deployments.toml and secrets from ../.env .
use near_api::{Account, NetworkConfig, SecretKey};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;

/// One client entry in deployments.toml.
#[derive(Deserialize, Clone)]
pub struct ClientConfig {
    pub account: String,
    pub owner: String,
    pub description: String,
    pub open_read_enabled: bool,
    pub admins: Vec<String>,
    pub products: Vec<String>,
    pub price_reporters: Vec<String>,
    pub authorized_callers: Vec<String>,
}

/// Top-level structure of deployments.toml — a flat map of client name → config.
#[derive(Deserialize)]
struct Deployments(HashMap<String, ClientConfig>);

/// Load client config, network, and signer.
pub async fn load_client(
    name: &str,
) -> eyre::Result<(ClientConfig, NetworkConfig, String, Arc<near_api::Signer>)> {
    // Load .env from the repo root.
    let dotenv_path = concat_path(&repo_root(), ".env");
    dotenvy::from_path(&dotenv_path)?;

    let private_key = env::var("NEAR_PRIVATE_KEY")
        .map_err(|_| eyre::eyre!("NEAR_PRIVATE_KEY not set in .env"))?;
    let private_key: SecretKey = private_key
        .parse()
        .map_err(|_| eyre::eyre!("invalid NEAR_PRIVATE_KEY in .env"))?;
    let rpc_url =
        env::var("NEAR_RPC_URL").map_err(|_| eyre::eyre!("NEAR_RPC_URL not set in .env"))?;

    // Load deployments.toml.
    let toml_path = concat_path(&repo_root(), "deployments.toml");
    let toml_content = std::fs::read_to_string(&toml_path)?;
    let deployments: Deployments = toml::from_str(&toml_content)?;

    let project = deployments
        .0
        .get(name)
        .ok_or_else(|| eyre::eyre!("client '{name}' not found in {toml_path}"))?;

    let network_config = NetworkConfig::from_rpc_url("near", rpc_url.parse()?);

    let public_key = private_key.public_key();
    let signer = near_api::Signer::from_secret_key(private_key)?;

    // Verify the private key belongs to the account.
    let account: near_api::AccountId = project.account.parse()?;
    let keys = Account(account.clone())
        .list_keys()
        .fetch_from(&network_config)
        .await?;
    let has_key = keys.data.iter().any(|(pk, _)| pk == &public_key);
    if !has_key {
        eyre::bail!(
            "private key does not belong to {account} — add key {} to the account first",
            public_key,
        );
    }

    Ok((project.clone(), network_config, rpc_url, signer))
}

/// Load network config and signer for feed submission.
/// The account is passed directly, not looked up from deployments.toml.
pub async fn load_feed_client()
-> eyre::Result<(NetworkConfig, near_api::AccountId, Arc<near_api::Signer>)> {
    let dotenv_path = concat_path(&repo_root(), ".env");
    dotenvy::from_path(&dotenv_path)?;

    let private_key = env::var("NEAR_PRICE_REPORTER_PRIVATE_KEY")
        .map_err(|_| eyre::eyre!("NEAR_PRICE_REPORTER_PRIVATE_KEY not set in .env"))?;
    let private_key: SecretKey = private_key
        .parse()
        .map_err(|_| eyre::eyre!("invalid NEAR_PRICE_REPORTER_PRIVATE_KEY in .env"))?;
    let reporter_account: near_api::AccountId = env::var("NEAR_PRICE_REPORTER_ACCOUNT")
        .map_err(|_| eyre::eyre!("NEAR_PRICE_REPORTER_ACCOUNT not set in .env"))?
        .parse()?;
    let rpc_url =
        env::var("NEAR_RPC_URL").map_err(|_| eyre::eyre!("NEAR_RPC_URL not set in .env"))?;

    let network_config = NetworkConfig::from_rpc_url("near", rpc_url.parse()?);
    let signer = near_api::Signer::from_secret_key(private_key)?;

    Ok((network_config, reporter_account, signer))
}

/// Return the repo root directory (parent of scripts/).
fn repo_root() -> String {
    concat_path(&env::current_dir().unwrap().to_string_lossy(), "..")
}

/// Join two path segments with a `/`.
fn concat_path(base: &str, segment: &str) -> String {
    format!("{base}/{segment}")
}

/// Parse a required CLI flag value (e.g. `--client client_name`).
pub fn parse_flag(args: &[String], flag: &str) -> eyre::Result<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1).cloned())
        .ok_or_else(|| eyre::eyre!("{flag} is required"))
}
