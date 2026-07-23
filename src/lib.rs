mod events;
mod types;

use near_sdk::store::{IterableSet, LookupMap, LookupSet};
use near_sdk::{AccountId, BorshStorageKey, PanicOnDefault, env, near, require};

use events::ContractEvent;
use types::{FeedData, FeedUpdate, Flags, RoleUpdate};

// Max allowed drift into the future for a reported aggregated timestamp (seconds).
const MAX_FUTURE_DRIFT_THRESHOLD: u64 = 60;
// env::block_timestamp() returns nanoseconds; we work in seconds everywhere.
const NANOS_PER_SEC: u64 = 1_000_000_000;

#[derive(BorshStorageKey)]
#[near]
enum StorageKey {
    Feeds,
    Admin,
    Products,
    PriceReporters,
    AuthorizedCallers,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct MultiFeed {
    feeds: LookupMap<u32, FeedData>,
    flags: Flags,
    owner: AccountId,
    description: String,
    admin: LookupSet<AccountId>,
    products: LookupSet<AccountId>,
    price_reporters: LookupSet<AccountId>,
    authorized_callers: IterableSet<AccountId>,
}

#[near]
impl MultiFeed {
    // Initialize the contract with the given owner, role members, and read-access mode.
    // Reverts on duplicate addresses in any role list, or if `open_read_enabled` is
    // true while `authorized_callers` is non-empty.
    #[init]
    pub fn new(
        owner: AccountId,
        open_read_enabled: bool,
        description: String,
        admins: Vec<AccountId>,
        products: Vec<AccountId>,
        price_reporters: Vec<AccountId>,
        authorized_callers: Vec<AccountId>,
    ) -> Self {
        // Enforce: open-read mode requires no authorized callers;
        // gated-read mode allows an initial whitelist or deferred setup.
        require!(
            !open_read_enabled || authorized_callers.is_empty(),
            "Open read incompatible with authorized callers"
        );

        // Initialize flags: not paused; open_read determined by parameter.
        let flags = Flags::new(false, open_read_enabled);

        // Emit initialization events.
        ContractEvent::OwnershipTransferred {
            old_owner: None,
            new_owner: owner.clone(),
        }
        .emit();
        ContractEvent::OpenReadStatusChanged {
            status: open_read_enabled,
        }
        .emit();

        Self {
            feeds: LookupMap::new(StorageKey::Feeds),
            flags,
            owner,
            description,
            admin: init_lookup_set_role(StorageKey::Admin, admins, |account, status| {
                ContractEvent::AdminStatusChanged { account, status }
            }),
            products: init_lookup_set_role(StorageKey::Products, products, |account, status| {
                ContractEvent::ProductStatusChanged { account, status }
            }),
            price_reporters: init_lookup_set_role(
                StorageKey::PriceReporters,
                price_reporters,
                |account, status| ContractEvent::PriceReporterStatusChanged { account, status },
            ),
            authorized_callers: {
                let mut caller_set = IterableSet::new(StorageKey::AuthorizedCallers);
                for addr in authorized_callers {
                    require!(
                        caller_set.insert(addr.clone()),
                        "Already an authorized caller"
                    );
                    ContractEvent::AuthorizedCallerStatusChanged {
                        account: addr,
                        status: true,
                    }
                    .emit();
                }
                caller_set
            },
        }
    }

    // Batch price feed. Only callable by a price reporter.
    pub fn f(&mut self, #[serializer(borsh)] updates: Vec<FeedUpdate>) {
        require!(
            self.price_reporters
                .contains(&env::predecessor_account_id()),
            "Only a price reporter can call"
        );

        require!(updates.len() > 0, "Empty feed data");

        // Compute block time in seconds; reused for timestamp validation and onchain_ts.
        let now = env::block_timestamp() / NANOS_PER_SEC;
        let future_bound = now + MAX_FUTURE_DRIFT_THRESHOLD;

        let mut feed_ids = Vec::with_capacity(updates.len());
        let mut prices = Vec::with_capacity(updates.len());
        let mut agg_ts = Vec::with_capacity(updates.len());

        for u in updates {
            // Validate agg_ts is strictly newer than stored and within future drift.
            let prev_agg_ts = self.feeds.get(&u.feed_id).map(|f| f.agg_ts).unwrap_or(0);
            require!(
                u.agg_ts > prev_agg_ts && u.agg_ts < future_bound,
                format!(
                    "Report timestamp out of bounds: feed_id={}, prev_agg_ts={}, now={}",
                    u.feed_id, prev_agg_ts, now
                )
            );

            // Write feed entry; onchain_ts set by the contract.
            self.feeds.insert(
                u.feed_id,
                FeedData {
                    price: u.price,
                    agg_ts: u.agg_ts,
                    onchain_ts: now,
                },
            );

            feed_ids.push(u.feed_id);
            prices.push(u.price);
            agg_ts.push(u.agg_ts);
        }

        // Emit event for the batch.
        ContractEvent::F {
            feed_ids,
            prices,
            agg_ts,
        }
        .emit();
    }

    // Transfer ownership to a new account. Only callable by the current owner.
    // Reverts if the new owner is the same as the current one.
    pub fn transfer_ownership(&mut self, new_owner: AccountId) {
        require!(
            env::predecessor_account_id() == self.owner,
            "Only the owner can call"
        );
        require!(
            new_owner != self.owner,
            "New owner must differ from the current owner"
        );

        let old_owner = self.owner.clone();
        self.owner = new_owner.clone();

        ContractEvent::OwnershipTransferred {
            old_owner: Some(old_owner),
            new_owner,
        }
        .emit();
    }

    // Update admin role memberships. Only callable by the owner.
    // Reverts on empty input or if any update is redundant.
    pub fn set_admins(&mut self, updates: Vec<RoleUpdate>) {
        require!(
            env::predecessor_account_id() == self.owner,
            "Only the owner can call"
        );
        require!(!updates.is_empty(), "Empty updates array");
        for update in updates {
            update_role_member(
                &mut self.admin,
                update.account,
                update.status,
                |account, status| ContractEvent::AdminStatusChanged { account, status },
            );
        }
    }

    // Update product role memberships. Only callable by the owner.
    // Reverts on empty input or if any update is redundant.
    pub fn set_products(&mut self, updates: Vec<RoleUpdate>) {
        require!(
            env::predecessor_account_id() == self.owner,
            "Only the owner can call"
        );
        require!(!updates.is_empty(), "Empty updates array");
        for update in updates {
            update_role_member(
                &mut self.products,
                update.account,
                update.status,
                |account, status| ContractEvent::ProductStatusChanged { account, status },
            );
        }
    }

    // Update price-reporter role memberships. Only callable by the owner.
    // Reverts on empty input or if any update is redundant.
    pub fn set_price_reporters(&mut self, updates: Vec<RoleUpdate>) {
        require!(
            env::predecessor_account_id() == self.owner,
            "Only the owner can call"
        );
        require!(!updates.is_empty(), "Empty updates array");
        for update in updates {
            update_role_member(
                &mut self.price_reporters,
                update.account,
                update.status,
                |account, status| ContractEvent::PriceReporterStatusChanged { account, status },
            );
        }
    }

    // Batch-update authorized-caller whitelist memberships.
    // Silently skips accounts already in the desired state (no revert on duplicates).
    // Callable even in open-read mode to pre-configure callers before
    // switching to gated reads.
    pub fn set_authorized_callers(&mut self, updates: Vec<RoleUpdate>) {
        require!(
            self.products.contains(&env::predecessor_account_id()),
            "Only a product can call"
        );
        require!(!updates.is_empty(), "Empty updates array");
        for update in updates {
            if update.status {
                if self.authorized_callers.insert(update.account.clone()) {
                    ContractEvent::AuthorizedCallerStatusChanged {
                        account: update.account,
                        status: true,
                    }
                    .emit();
                }
            } else {
                if self.authorized_callers.remove(&update.account) {
                    ContractEvent::AuthorizedCallerStatusChanged {
                        account: update.account,
                        status: false,
                    }
                    .emit();
                }
            }
        }
    }

    // Toggle the contract paused state.
    // Reverts if the contract is already in the target state.
    pub fn set_paused(&mut self, paused: bool) {
        require!(
            self.admin.contains(&env::predecessor_account_id()),
            "Only an admin can call"
        );
        require!(self.flags.is_paused() != paused, "Already in target state");
        self.flags.flip_paused();
        ContractEvent::PausedStatusChanged { status: paused }.emit();
    }

    // Toggle the open-read flag.
    // Reverts if the flag is already in the target state.
    pub fn set_open_read_status(&mut self, status: bool) {
        require!(
            env::predecessor_account_id() == self.owner,
            "Only the owner can call"
        );
        require!(
            self.flags.is_open_read() != status,
            "Already in target state"
        );
        self.flags.flip_open_read();
        ContractEvent::OpenReadStatusChanged { status }.emit();
    }

    // Return the feed matching `feed_id`, gated by read-access control.
    //
    // # Arguments
    // * `feed_id` - a `0x`-prefixed 8-hex-char string (EVM `bytes4`)
    //
    // # Panics
    // Panics if the contract is paused, the caller lacks read permission,
    // or the `feed_id` is malformed.
    //
    // # Returns
    // `Some(feed)` if the feed exists; `None` if absent.
    pub fn fetch(&self, feed_id: String) -> Option<FeedData> {
        self.only_read_access();
        self.feeds.get(&parse_feed_id(&feed_id)).cloned()
    }

    // Return feeds for a batch of feed IDs, gated by read-access control.
    //
    // # Arguments
    // * `feed_ids` - a list of `0x`-prefixed 8-hex-char strings (EVM `bytes4`)
    //
    // # Panics
    // Panics if the contract is paused, the caller lacks read permission,
    // or any `feed_id` is malformed.
    //
    // # Returns
    // A vector of the same length; each element is `Some(feed)` if the
    // feed exists, `None` if absent.
    pub fn fetch_batch(&self, feed_ids: Vec<String>) -> Vec<Option<FeedData>> {
        self.only_read_access();
        feed_ids
            .iter()
            .map(|id| self.feeds.get(&parse_feed_id(id)).cloned())
            .collect()
    }

    // Return the current contract owner.
    pub fn get_owner(&self) -> AccountId {
        self.owner.clone()
    }

    // Return the contract description.
    pub fn description(&self) -> String {
        self.description.clone()
    }

    // Return the number of decimals for price values.
    pub fn decimals(&self) -> u8 {
        18
    }

    // Return the contract version.
    pub fn version(&self) -> u8 {
        1
    }

    // Return whether the given account holds the admin role.
    pub fn is_admin(&self, account: AccountId) -> bool {
        self.admin.contains(&account)
    }

    // Return whether the given account holds the product role.
    pub fn is_product(&self, account: AccountId) -> bool {
        self.products.contains(&account)
    }

    // Return whether the given account holds the price-reporter role.
    pub fn is_price_reporter(&self, account: AccountId) -> bool {
        self.price_reporters.contains(&account)
    }

    pub fn is_authorized_caller(&self, account: AccountId) -> bool {
        self.authorized_callers.contains(&account)
    }

    // Return whether the contract is currently paused.
    pub fn is_paused(&self) -> bool {
        self.flags.is_paused()
    }

    // Return whether reads are open (permissionless) or gated by the authorized-caller whitelist.
    pub fn is_open_read(&self) -> bool {
        self.flags.is_open_read()
    }

    // Return all accounts in the authorized-caller whitelist.
    pub fn get_authorized_callers(&self) -> Vec<AccountId> {
        self.authorized_callers.iter().cloned().collect()
    }

    // Enforce read-access control: reverts if paused or the caller lacks permission.
    // Short-circuit: paused → open-read → authorized caller.
    // Flags checks are in-memory (cheap); authorized_callers.contains is a storage read (expensive).
    fn only_read_access(&self) {
        if self.flags.is_paused() {
            env::panic_str("Enforced paused");
        }
        // (a) open-read && not paused → `flags == 1`, cheapest in-memory compare, first;
        // (b) authorized caller → storage read, second.
        if self.flags.is_open_read_when_unpaused()
            || self
                .authorized_callers
                .contains(&env::predecessor_account_id())
        {
            return;
        }
        env::panic_str("Only authorized caller allowed");
    }
}

// Parse a hex-encoded feed ID string (e.g. "0x0000002a") into a u32.
// Reverts if the input is not prefixed with "0x" or is not exactly 8 hex digits.
fn parse_feed_id(s: &str) -> u32 {
    require!(
        s.len() == 10 && s.starts_with("0x"),
        "Feed id must be 0x followed by exactly 8 hex digits"
    );
    let Ok(v) = u32::from_str_radix(&s[2..], 16) else {
        env::panic_str("Invalid hex feed id")
    };
    v
}

// Insert or remove a role member from a `LookupSet<AccountId>`.
// Emits an event on success; reverts if the account's current status already
// matches the target (prevents duplicate adds or redundant removes).
// Not applicable to `IterableSet`-based roles (e.g. authorized callers).
fn update_role_member(
    set: &mut LookupSet<AccountId>,
    account: AccountId,
    status: bool,
    event_fn: impl Fn(AccountId, bool) -> ContractEvent,
) {
    if status {
        require!(set.insert(account.clone()), "Already in role");
        event_fn(account, status).emit();
    } else {
        require!(set.remove(&account), "Not in role");
        event_fn(account, status).emit();
    }
}

// Initialize a `LookupSet<AccountId>` role from a list of addresses,
// inserting each with `status = true` and emitting an event per insertion.
// Reverts on duplicate addresses.
fn init_lookup_set_role(
    key: StorageKey,
    addrs: Vec<AccountId>,
    event_fn: impl Fn(AccountId, bool) -> ContractEvent,
) -> LookupSet<AccountId> {
    let mut set = LookupSet::new(key);
    for addr in addrs {
        update_role_member(&mut set, addr, true, &event_fn);
    }
    set
}

#[cfg(test)]
mod tests {
    use crate::{ContractEvent, MultiFeed};
    use near_sdk::{AccountId, serde_json, testing_env};

    fn alice() -> AccountId {
        "alice.near".parse().unwrap()
    }

    fn bob() -> AccountId {
        "bob.near".parse().unwrap()
    }

    fn owner() -> AccountId {
        "owner.near".parse().unwrap()
    }

    fn set_caller(account: AccountId) {
        testing_env!(
            near_sdk::test_utils::VMContextBuilder::new()
                .predecessor_account_id(account)
                .build()
        );
    }

    fn assert_event_log(log: &str, expected: ContractEvent) {
        fn event_to_json(event: ContractEvent) -> serde_json::Value {
            const STANDARD: &str = "multi-feed";
            const VERSION: &str = "1.0.0";
            use serde_json::json;

            fn base(event_name: &str, data: serde_json::Value) -> serde_json::Value {
                json!({
                    "standard": STANDARD,
                    "version": VERSION,
                    "event": event_name,
                    "data": data,
                })
            }

            match event {
                ContractEvent::OwnershipTransferred {
                    old_owner,
                    new_owner,
                } => base(
                    "ownership_transferred",
                    json!({
                        "old_owner": old_owner.as_ref().map(|o: &AccountId| o.to_string()),
                        "new_owner": new_owner.to_string(),
                    }),
                ),
                ContractEvent::AdminStatusChanged { account, status } => base(
                    "admin_status_changed",
                    json!({
                        "account": account.to_string(),
                        "status": status,
                    }),
                ),
                ContractEvent::ProductStatusChanged { account, status } => base(
                    "product_status_changed",
                    json!({
                        "account": account.to_string(),
                        "status": status,
                    }),
                ),
                ContractEvent::PriceReporterStatusChanged { account, status } => base(
                    "price_reporter_status_changed",
                    json!({
                        "account": account.to_string(),
                        "status": status,
                    }),
                ),
                ContractEvent::AuthorizedCallerStatusChanged { account, status } => base(
                    "authorized_caller_status_changed",
                    json!({
                        "account": account.to_string(),
                        "status": status,
                    }),
                ),
                ContractEvent::OpenReadStatusChanged { status } => {
                    base("open_read_status_changed", json!({ "status": status }))
                }
                ContractEvent::PausedStatusChanged { status } => {
                    base("paused_status_changed", json!({ "status": status }))
                }
                ContractEvent::F {
                    feed_ids,
                    prices,
                    agg_ts,
                } => base(
                    "f",
                    json!({
                        "i": feed_ids,
                        "p": prices,
                        "t": agg_ts,
                    }),
                ),
            }
        }

        let expected_json = event_to_json(expected);
        let json_str = log
            .strip_prefix("EVENT_JSON:")
            .expect("Missing EVENT_JSON prefix");
        let actual: serde_json::Value = serde_json::from_str(json_str).expect("Invalid JSON");
        assert_eq!(actual, expected_json, "Event mismatch");
    }

    fn init_contract() -> MultiFeed {
        MultiFeed::new(
            owner(),
            false,
            "Test".to_string(),
            vec![],
            vec![],
            vec![],
            vec![],
        )
    }

    mod parse_feed_id {
        use crate::parse_feed_id;

        #[test]
        fn valid_zero() {
            assert_eq!(parse_feed_id("0x00000000"), 0);
        }

        #[test]
        fn valid_one() {
            assert_eq!(parse_feed_id("0x00000001"), 1);
        }

        #[test]
        fn valid_max() {
            assert_eq!(parse_feed_id("0xffffffff"), u32::MAX);
        }

        #[test]
        fn valid_arbitrary() {
            assert_eq!(parse_feed_id("0x0000002a"), 42);
        }

        #[test]
        fn valid_uppercase() {
            assert_eq!(parse_feed_id("0xABCDEF12"), 0xABCDEF12);
        }

        #[test]
        fn valid_lowercase() {
            assert_eq!(parse_feed_id("0xabcdef12"), 0xABCDEF12);
        }

        #[test]
        fn valid_mixed_case() {
            assert_eq!(parse_feed_id("0xAbCdEf12"), 0xABCDEF12);
        }

        #[test]
        #[should_panic(expected = "Feed id must be 0x followed by exactly 8 hex digits")]
        fn missing_prefix() {
            parse_feed_id("0000002a");
        }

        #[test]
        #[should_panic(expected = "Feed id must be 0x followed by exactly 8 hex digits")]
        fn uppercase_prefix() {
            parse_feed_id("0X0000002a");
        }

        #[test]
        #[should_panic(expected = "Feed id must be 0x followed by exactly 8 hex digits")]
        fn too_short() {
            parse_feed_id("0x00002a");
        }

        #[test]
        #[should_panic(expected = "Feed id must be 0x followed by exactly 8 hex digits")]
        fn too_long() {
            parse_feed_id("0x000000000");
        }

        #[test]
        #[should_panic(expected = "Invalid hex feed id")]
        fn invalid_hex_chars() {
            parse_feed_id("0x00gh0000");
        }

        #[test]
        #[should_panic(expected = "Feed id must be 0x followed by exactly 8 hex digits")]
        fn empty() {
            parse_feed_id("");
        }
    }

    mod update_role_member {
        use super::{alice, assert_event_log, bob};
        use crate::{ContractEvent, update_role_member};
        use near_sdk::store::LookupSet;
        use near_sdk::test_utils::get_logs;

        #[test]
        fn add() {
            let mut set = LookupSet::new(crate::StorageKey::Admin);
            update_role_member(&mut set, alice(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            assert!(set.contains(&alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn add_duplicate() {
            let mut set = LookupSet::new(crate::StorageKey::Admin);
            update_role_member(&mut set, alice(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            update_role_member(&mut set, alice(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
        }

        #[test]
        fn remove() {
            let mut set = LookupSet::new(crate::StorageKey::Admin);
            update_role_member(&mut set, alice(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            update_role_member(&mut set, alice(), false, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            assert!(!set.contains(&alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[1],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Not in role")]
        fn remove_not_in_role() {
            let mut set = LookupSet::new(crate::StorageKey::Admin);
            update_role_member(&mut set, alice(), false, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
        }

        #[test]
        fn add_bob_after_remove_alice() {
            let mut set = LookupSet::new(crate::StorageKey::Admin);
            update_role_member(&mut set, alice(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            update_role_member(&mut set, alice(), false, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            update_role_member(&mut set, bob(), true, |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            assert!(!set.contains(&alice()));
            assert!(set.contains(&bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[2],
                ContractEvent::PriceReporterStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }
    }

    mod init_lookup_set_role {
        use super::{alice, assert_event_log, bob};
        use crate::{ContractEvent, StorageKey, init_lookup_set_role};
        use near_sdk::test_utils::get_logs;

        #[test]
        fn empty() {
            let set = init_lookup_set_role(StorageKey::Admin, vec![], |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            assert!(!set.contains(&alice()));
        }

        #[test]
        fn single() {
            let set = init_lookup_set_role(StorageKey::Admin, vec![alice()], |account, status| {
                ContractEvent::PriceReporterStatusChanged { account, status }
            });
            assert!(set.contains(&alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
        }

        #[test]
        fn multiple() {
            let set = init_lookup_set_role(
                StorageKey::Admin,
                vec![alice(), bob()],
                |account, status| ContractEvent::PriceReporterStatusChanged { account, status },
            );
            assert!(set.contains(&alice()));
            assert!(set.contains(&bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::PriceReporterStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate() {
            init_lookup_set_role(
                StorageKey::Admin,
                vec![alice(), alice()],
                |account, status| ContractEvent::PriceReporterStatusChanged { account, status },
            );
        }
    }

    mod new {
        use super::{alice, assert_event_log, bob, owner};
        use crate::{ContractEvent, MultiFeed};
        use near_sdk::test_utils::get_logs;

        #[test]
        fn gated_mode_init() {
            let contract = MultiFeed::new(
                owner(),
                false,
                "Gated Mode".to_string(),
                vec![alice()],
                vec![bob()],
                vec![alice()],
                vec![alice(), bob()],
            );
            assert_eq!(contract.get_owner(), owner());
            assert_eq!(contract.description(), "Gated Mode");
            assert!(!contract.is_paused());
            assert!(!contract.is_open_read());
            assert!(contract.is_admin(alice()));
            assert!(contract.is_product(bob()));
            assert!(contract.is_price_reporter(alice()));
            assert!(contract.is_authorized_caller(alice()));
            assert!(contract.is_authorized_caller(bob()));

            let callers = contract.get_authorized_callers();
            assert_eq!(callers.len(), 2);
            assert!(callers.contains(&alice()));
            assert!(callers.contains(&bob()));

            let logs = get_logs();
            assert_eq!(logs.len(), 7);
            // Init events.
            assert_event_log(
                &logs[0],
                ContractEvent::OwnershipTransferred {
                    old_owner: None,
                    new_owner: owner(),
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::OpenReadStatusChanged { status: false },
            );
            // Role events.
            assert_event_log(
                &logs[2],
                ContractEvent::AdminStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[3],
                ContractEvent::ProductStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[4],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[5],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[6],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        fn open_read_mode_init() {
            let contract = MultiFeed::new(
                owner(),
                true,
                "Open Read Mode".to_string(),
                vec![],
                vec![],
                vec![],
                vec![],
            );
            assert!(contract.is_open_read());
            assert!(!contract.is_paused());
            assert_eq!(contract.get_owner(), owner());
            assert_eq!(contract.description(), "Open Read Mode");
            assert!(contract.get_authorized_callers().is_empty());

            let logs = get_logs();
            assert_eq!(logs.len(), 2);
            assert_event_log(
                &logs[0],
                ContractEvent::OwnershipTransferred {
                    old_owner: None,
                    new_owner: owner(),
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::OpenReadStatusChanged { status: true },
            );
        }

        #[test]
        #[should_panic(expected = "Open read incompatible with authorized callers")]
        fn open_read_with_callers() {
            MultiFeed::new(
                owner(),
                true,
                "Invalid".to_string(),
                vec![],
                vec![],
                vec![],
                vec![alice()],
            );
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_admin() {
            MultiFeed::new(
                owner(),
                false,
                "Test".to_string(),
                vec![alice(), alice()],
                vec![],
                vec![],
                vec![],
            );
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_product() {
            MultiFeed::new(
                owner(),
                false,
                "Test".to_string(),
                vec![],
                vec![bob(), bob()],
                vec![],
                vec![],
            );
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_price_reporter() {
            MultiFeed::new(
                owner(),
                false,
                "Test".to_string(),
                vec![],
                vec![],
                vec![alice(), alice()],
                vec![],
            );
        }

        #[test]
        #[should_panic(expected = "Already an authorized caller")]
        fn duplicate_authorized_caller() {
            MultiFeed::new(
                owner(),
                false,
                "Test".to_string(),
                vec![],
                vec![],
                vec![],
                vec![bob(), bob()],
            );
        }
    }

    mod transfer_ownership {
        use super::init_contract;
        use super::{alice, assert_event_log, owner, set_caller};
        use crate::ContractEvent;
        use near_sdk::test_utils::get_logs;

        #[test]
        fn transfer() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.transfer_ownership(alice());
            assert_eq!(contract.get_owner(), alice());

            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::OwnershipTransferred {
                    old_owner: Some(owner()),
                    new_owner: alice(),
                },
            );
        }

        #[test]
        #[should_panic(expected = "Only the owner can call")]
        fn non_owner_cannot_transfer() {
            let mut contract = init_contract();
            set_caller(alice());
            contract.transfer_ownership(alice());
        }

        #[test]
        #[should_panic(expected = "New owner must differ from the current owner")]
        fn same_owner() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.transfer_ownership(owner());
        }
    }

    mod set_admins {
        use super::{alice, assert_event_log, bob, init_contract, owner, set_caller};
        use crate::{ContractEvent, RoleUpdate};
        use near_sdk::test_utils::get_logs;

        #[test]
        fn add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            assert!(contract.is_admin(alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::AdminStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
        }

        #[test]
        fn batch_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            assert!(contract.is_admin(alice()));
            assert!(contract.is_admin(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::AdminStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::AdminStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        fn remove() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: false,
            }]);
            assert!(!contract.is_admin(alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[1],
                ContractEvent::AdminStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
        }

        #[test]
        fn batch_remove() {
            let mut contract = init_contract();
            set_caller(owner());
            // Add both first.
            contract.set_admins(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            // Remove both in one call.
            contract.set_admins(vec![
                RoleUpdate {
                    account: alice(),
                    status: false,
                },
                RoleUpdate {
                    account: bob(),
                    status: false,
                },
            ]);
            assert!(!contract.is_admin(alice()));
            assert!(!contract.is_admin(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[2],
                ContractEvent::AdminStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
            assert_event_log(
                &logs[3],
                ContractEvent::AdminStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Only the owner can call")]
        fn non_owner() {
            let mut contract = init_contract();
            set_caller(alice());
            contract.set_admins(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
        }

        #[test]
        #[should_panic(expected = "Empty updates array")]
        fn empty_updates() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![]);
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
        }

        #[test]
        #[should_panic(expected = "Not in role")]
        fn remove_not_in_role() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_admins(vec![RoleUpdate {
                account: alice(),
                status: false,
            }]);
        }
    }

    mod set_products {
        use super::init_contract;
        use super::{alice, assert_event_log, bob, owner, set_caller};
        use crate::{ContractEvent, RoleUpdate};
        use near_sdk::test_utils::get_logs;

        #[test]
        fn add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            assert!(contract.is_product(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::ProductStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        fn remove() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: false,
            }]);
            assert!(!contract.is_product(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[1],
                ContractEvent::ProductStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Only the owner can call")]
        fn non_owner() {
            let mut contract = init_contract();
            set_caller(alice());
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
        }

        #[test]
        #[should_panic(expected = "Empty updates array")]
        fn empty_updates() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![]);
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            contract.set_products(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
        }

        #[test]
        fn batch_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            assert!(contract.is_product(alice()));
            assert!(contract.is_product(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::ProductStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::ProductStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Not in role")]
        fn remove_not_in_role() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![RoleUpdate {
                account: alice(),
                status: false,
            }]);
        }

        #[test]
        fn batch_remove() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_products(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            contract.set_products(vec![
                RoleUpdate {
                    account: alice(),
                    status: false,
                },
                RoleUpdate {
                    account: bob(),
                    status: false,
                },
            ]);
            assert!(!contract.is_product(alice()));
            assert!(!contract.is_product(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[2],
                ContractEvent::ProductStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
            assert_event_log(
                &logs[3],
                ContractEvent::ProductStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }
    }

    mod set_price_reporters {
        use super::init_contract;
        use super::{alice, assert_event_log, bob, owner, set_caller};
        use crate::{ContractEvent, RoleUpdate};
        use near_sdk::test_utils::get_logs;

        #[test]
        fn add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            assert!(contract.is_price_reporter(alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
        }

        #[test]
        fn remove() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: false,
            }]);
            assert!(!contract.is_price_reporter(alice()));
            let logs = get_logs();
            assert_event_log(
                &logs[1],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Only the owner can call")]
        fn non_owner() {
            let mut contract = init_contract();
            set_caller(alice());
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
        }

        #[test]
        #[should_panic(expected = "Empty updates array")]
        fn empty_updates() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![]);
        }

        #[test]
        #[should_panic(expected = "Already in role")]
        fn duplicate_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: true,
            }]);
        }

        #[test]
        fn batch_add() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            assert!(contract.is_price_reporter(alice()));
            assert!(contract.is_price_reporter(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::PriceReporterStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Not in role")]
        fn remove_not_in_role() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![RoleUpdate {
                account: alice(),
                status: false,
            }]);
        }

        #[test]
        fn batch_remove() {
            let mut contract = init_contract();
            set_caller(owner());
            contract.set_price_reporters(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            contract.set_price_reporters(vec![
                RoleUpdate {
                    account: alice(),
                    status: false,
                },
                RoleUpdate {
                    account: bob(),
                    status: false,
                },
            ]);
            assert!(!contract.is_price_reporter(alice()));
            assert!(!contract.is_price_reporter(bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[2],
                ContractEvent::PriceReporterStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
            assert_event_log(
                &logs[3],
                ContractEvent::PriceReporterStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }
    }

    mod set_authorized_callers {
        use super::{alice, assert_event_log, bob, owner, set_caller};
        use crate::{ContractEvent, MultiFeed, RoleUpdate};
        use near_sdk::test_utils::get_logs;

        fn init_contract_with_product() -> MultiFeed {
            MultiFeed::new(
                owner(),
                false,
                "Test".to_string(),
                vec![],
                vec![alice()],
                vec![],
                vec![],
            )
        }

        #[test]
        fn add() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            assert!(contract.is_authorized_caller(bob()));
            let callers = contract.get_authorized_callers();
            assert_eq!(callers.len(), 1);
            assert!(callers.contains(&bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        fn remove() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: false,
            }]);
            assert!(!contract.is_authorized_caller(bob()));
            let callers = contract.get_authorized_callers();
            assert!(callers.is_empty());
            let logs = get_logs();
            assert_event_log(
                &logs[1],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }

        #[test]
        #[should_panic(expected = "Only a product can call")]
        fn non_product() {
            let mut contract = init_contract_with_product();
            set_caller(owner());
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
        }

        #[test]
        #[should_panic(expected = "Empty updates array")]
        fn empty_updates() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![]);
        }

        #[test]
        fn duplicate_add_skipped() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: true,
            }]);
            assert!(contract.is_authorized_caller(bob()));
            let callers = contract.get_authorized_callers();
            assert_eq!(callers.len(), 1);
            assert!(callers.contains(&bob()));
            let logs = get_logs();
            assert_eq!(logs.len(), 1);
        }

        #[test]
        fn remove_not_in_role_skipped() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![RoleUpdate {
                account: bob(),
                status: false,
            }]);
            let callers = contract.get_authorized_callers();
            assert!(callers.is_empty());
            let logs = get_logs();
            assert!(logs.is_empty());
        }

        #[test]
        fn batch_add() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            assert!(contract.is_authorized_caller(alice()));
            assert!(contract.is_authorized_caller(bob()));
            let callers = contract.get_authorized_callers();
            assert_eq!(callers.len(), 2);
            assert!(callers.contains(&alice()));
            assert!(callers.contains(&bob()));
            let logs = get_logs();
            assert_event_log(
                &logs[0],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: alice(),
                    status: true,
                },
            );
            assert_event_log(
                &logs[1],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: bob(),
                    status: true,
                },
            );
        }

        #[test]
        fn batch_remove() {
            let mut contract = init_contract_with_product();
            set_caller(alice());
            contract.set_authorized_callers(vec![
                RoleUpdate {
                    account: alice(),
                    status: true,
                },
                RoleUpdate {
                    account: bob(),
                    status: true,
                },
            ]);
            contract.set_authorized_callers(vec![
                RoleUpdate {
                    account: alice(),
                    status: false,
                },
                RoleUpdate {
                    account: bob(),
                    status: false,
                },
            ]);
            assert!(!contract.is_authorized_caller(alice()));
            assert!(!contract.is_authorized_caller(bob()));
            let callers = contract.get_authorized_callers();
            assert!(callers.is_empty());
            let logs = get_logs();
            assert_event_log(
                &logs[2],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: alice(),
                    status: false,
                },
            );
            assert_event_log(
                &logs[3],
                ContractEvent::AuthorizedCallerStatusChanged {
                    account: bob(),
                    status: false,
                },
            );
        }
    }
}
