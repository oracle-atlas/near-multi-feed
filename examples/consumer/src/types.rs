// Data types for the consumer contract's own ABI.

use crate::ext::FeedData;
use near_sdk::json_types::U128;
use near_sdk::near;

// JSON view of a fetched feed for this contract's own ABI output; price is
// a decimal string (U128) to avoid IEEE-754 precision loss in JS consumers.
#[near(serializers = [json])]
pub struct FeedDataView {
    pub price: U128,
    pub agg_ts: u64,
    pub onchain_ts: u64,
}

impl From<FeedData> for FeedDataView {
    fn from(f: FeedData) -> Self {
        Self {
            price: U128(f.price),
            agg_ts: f.agg_ts,
            onchain_ts: f.onchain_ts,
        }
    }
}
