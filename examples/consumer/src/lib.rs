// Example consumer contract: demonstrates how to read prices from a deployed
// `multi-feed` multi-feed via cross-contract calls to `fetch` and `fetch_batch`.
use near_sdk::{AccountId, Gas, NearToken, PanicOnDefault, Promise, env, near};

mod ext;
use ext::{FeedData, ext_multi_feed};

const FETCH_GAS: Gas = Gas::from_tgas(5);
const CALLBACK_GAS: Gas = Gas::from_tgas(5);

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MultiFeedConsumer {
    // Address of the deployed multi-feed contract.
    multi_feed: AccountId,
}

#[near]
impl MultiFeedConsumer {
    #[init]
    pub fn new(multi_feed: AccountId) -> Self {
        Self { multi_feed }
    }

    // Return the configured multi-feed address.
    pub fn get_multi_feed(&self) -> AccountId {
        self.multi_feed.clone()
    }

    // Fetch a single feed from multi-feed. Resolves to `Option<FeedData>`.
    pub fn get_price(&self, feed_id: String) -> Promise {
        ext_multi_feed::ext(self.multi_feed.clone())
            .with_static_gas(FETCH_GAS)
            .with_attached_deposit(NearToken::from_yoctonear(0))
            .fetch(feed_id)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_price(),
            )
    }

    // Fetch a batch of feeds from multi-feed. Resolves to `Vec<Option<FeedData>>`.
    pub fn get_prices(&self, feed_ids: Vec<String>) -> Promise {
        ext_multi_feed::ext(self.multi_feed.clone())
            .with_static_gas(FETCH_GAS)
            .with_attached_deposit(NearToken::from_yoctonear(0))
            .fetch_batch(feed_ids)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_prices(),
            )
    }

    // Callback: forward the single-feed result to the caller.
    #[private]
    pub fn on_price(&self, #[callback_unwrap] feed: Option<FeedData>) -> Option<FeedData> {
        feed
    }

    // Callback: forward the batch result to the caller.
    #[private]
    pub fn on_prices(
        &self,
        #[callback_unwrap] feeds: Vec<Option<FeedData>>,
    ) -> Vec<Option<FeedData>> {
        feeds
    }
}
