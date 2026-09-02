// Example consumer contract: demonstrates how to read prices from a deployed
// `multi-feed` contract via cross-contract calls to `fetch` and `fetch_batch`.
mod ext;
mod types;

use ext::{FeedData, ext_multi_feed};
use near_sdk::{AccountId, Gas, PanicOnDefault, Promise, env, near};
pub use types::FeedDataView;

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

    // Fetch a single feed from multi-feed. Resolves to `Option<FeedDataView>`.
    pub fn get_price(&self, feed_id: String) -> Promise {
        ext_multi_feed::ext(self.multi_feed.clone())
            .with_static_gas(FETCH_GAS)
            .fetch(feed_id)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_price(),
            )
    }

    // Fetch a batch of feeds from multi-feed. Resolves to `Vec<Option<FeedDataView>>`.
    pub fn get_prices(&self, feed_ids: Vec<String>) -> Promise {
        ext_multi_feed::ext(self.multi_feed.clone())
            .with_static_gas(FETCH_GAS)
            .fetch_batch(feed_ids)
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(CALLBACK_GAS)
                    .on_prices(),
            )
    }

    // Callback: decode the oracle's borsh result and forward it to the
    // caller as this contract's JSON view.
    #[private]
    pub fn on_price(
        &self,
        #[callback_unwrap]
        #[serializer(borsh)]
        feed: Option<FeedData>,
    ) -> Option<FeedDataView> {
        feed.map(FeedDataView::from)
    }

    // Callback: decode the oracle's borsh batch result and forward it to
    // the caller as this contract's JSON view.
    #[private]
    pub fn on_prices(
        &self,
        #[callback_unwrap]
        #[serializer(borsh)]
        feeds: Vec<Option<FeedData>>,
    ) -> Vec<Option<FeedDataView>> {
        feeds
            .into_iter()
            .map(|feed| feed.map(FeedDataView::from))
            .collect()
    }
}
