use near_sdk::{AccountId, near};

#[near(event_json(standard = "multi-feed"))]
pub enum ContractEvent {
    #[event_version("1.0.0")]
    OwnershipTransferred {
        old_owner: Option<AccountId>,
        new_owner: AccountId,
    },
    #[event_version("1.0.0")]
    AdminStatusChanged { account: AccountId, status: bool },
    #[event_version("1.0.0")]
    ProductStatusChanged { account: AccountId, status: bool },
    #[event_version("1.0.0")]
    PriceReporterStatusChanged { account: AccountId, status: bool },
    #[event_version("1.0.0")]
    AuthorizedCallerStatusChanged { account: AccountId, status: bool },
    #[event_version("1.0.0")]
    OpenReadStatusChanged { status: bool },
    #[event_version("1.0.0")]
    PausedStatusChanged { status: bool },

    // ──── High-frequency feed event: aggressively minimized for gas ────
    // Fires on every price feed, so we trade readability for fewer log bytes:
    //   - variant name `F`  → serializes to "event":"f"
    //   - single-char JSON keys: i / p / t  (Rust field names stay readable)
    // Parallel arrays (not an array of structs) so keys appear once, not per entry.
    #[event_version("1.0.0")]
    F {
        #[serde(rename = "i")]
        feed_ids: Vec<u32>,
        #[serde(rename = "p")]
        prices: Vec<u128>,
        #[serde(rename = "t")]
        agg_ts: Vec<u64>,
    },
}
