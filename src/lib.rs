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

    // Return whether the given account is in the authorized-caller whitelist.
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

    // Return all accounts in the authorized-caller whitelist.
    pub fn get_authorized_callers(&self) -> Vec<AccountId> {
        self.authorized_callers.iter().cloned().collect()
    }

    // Transfer contract ownership to a new account.
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

    // Batch-update admin role memberships.
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

    // Batch-update product role memberships.
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

    // Batch-update price-reporter role memberships.
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
