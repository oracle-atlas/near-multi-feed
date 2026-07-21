use near_sdk::{AccountId, near};

// Role membership update: account + desired status (add = true, remove = false).
#[near(serializers = [json])]
pub struct RoleUpdate {
    pub account: AccountId,
    pub status: bool,
}

// Single price feed entry submitted by a reporter.
#[near(serializers = [json])]
pub struct FeedUpdate {
    pub feed_id: u32,
    pub price: u128,
    pub agg_ts: u64,
}
