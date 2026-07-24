// External interface to the `multi-feed` oracle contract.
use near_sdk::{ext_contract, near};

// Mirror of the multi-feed's JSON `FeedData` view (price + agg_ts + onchain_ts).
#[near(serializers = [json])]
#[derive(Clone)]
pub struct FeedData {
    pub price: u128,
    pub agg_ts: u64,
    pub onchain_ts: u64,
}

#[ext_contract(ext_multi_feed)]
#[allow(dead_code)]
pub trait MultiFeed {
    fn fetch(&self, feed_id: String) -> Option<FeedData>;
    fn fetch_batch(&self, feed_ids: Vec<String>) -> Vec<Option<FeedData>>;
}
