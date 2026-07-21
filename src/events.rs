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
    #[event_version("1.0.0")]
    PricesFed { feed_ids: Vec<u32> },
}
