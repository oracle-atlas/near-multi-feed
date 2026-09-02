# Multi Feed (NEAR)

A **stateful, on-chain** price feed oracle for NEAR, built to be read by other on-chain contracts — DEXs, lending protocols, derivatives, and similar.

Authorized reporters submit batched prices through `f`, a borsh-encoded feed entrypoint; the contract validates each timestamp and stores the latest price per feed. Consumers read those prices through `fetch` / `fetch_batch`, subject to the read mode: **open read** lets anyone query, while **gated read** restricts access to the authorized-caller whitelist. The getters are read-only, so they can also be called for **free** via an RPC view call (off-chain frontends and indexers). On-chain consumers call them via a cross-contract call and read the result in a callback — see [Building a consumer contract](#building-a-consumer-contract).

## Methods

The two feed getters are the main read API; the rest are role and status views.

| Method | Returns | Description |
|--------|---------|-------------|
| `fetch(feed_id)` | `Option<FeedData>` | Single feed by ID — `None` if absent |
| `fetch_batch(feed_ids)` | `Vec<Option<FeedData>>` | Batch feed lookup — each element is `None` if absent; empty `feed_ids` returns an empty vector |
| `get_owner()` | `AccountId` | Contract owner |
| `description()` | `String` | Human-readable contract description |
| `decimals()` | `u8` | Price decimals |
| `version()` | `u8` | Contract version |
| `is_admin(account)` | `bool` | Check admin role |
| `is_product(account)` | `bool` | Check product role |
| `is_price_reporter(account)` | `bool` | Check price reporter role |
| `is_authorized_caller(account)` | `bool` | Check authorized caller |
| `is_paused()` | `bool` | Whether the contract is paused |
| `is_open_read()` | `bool` | Whether open-read is enabled |
| `get_authorized_callers()` | `Vec<AccountId>` | All authorized callers |

- `feed_id`: which feed to select, a `0x`-prefixed 8-hex-char string (EVM `bytes4`). Feed ids are assigned by the operator — get the id-to-asset registry from the operator.

> **Failure modes are split:**
> - **Panics (reverts)** when the contract is paused, when the caller lacks read permission in gated mode, or when a `feed_id` is malformed. Treat an RPC error as "not allowed to read".
> - **Returns `None`** when reads are allowed but the feed is absent. Callers can fall back or skip.

The getters return **borsh-encoded** results (`#[result_serializer(borsh)]`), which are cheaper to serialize and deserialize than JSON — saving gas on cross-contract calls. The same bytes reach both RPC view callers and cross-contract callbacks. `FeedData` borsh layout:

```text
Option<FeedData> = [variant: 1 byte (0 = None, 1 = Some)][FeedData]
FeedData = [price: u128 — 16 bytes LE][agg_ts: u64 — 8 bytes LE][onchain_ts: u64 — 8 bytes LE]
Vec<Option<FeedData>> = [length: u32 LE][entry]... (fetch_batch)
```

## Read modes

Two read modes, controlled by the owner:

- **Open read** (`is_open_read() == true`) — anyone can call `fetch` / `fetch_batch`. Set at init with `open_read_enabled: true`.
- **Gated read** (`is_open_read() == false`) — only addresses in the authorized-caller whitelist can read. The whitelist is managed by products.

Either mode is blocked when the contract is paused.

## Access control

- **Owner**: full control — transfer ownership, manage roles, toggle open-read.
- **Admin**: can pause / unpause.
- **Product**: can manage the authorized-caller whitelist.
- **Price Reporter**: can submit feeds via `f`.
- **Authorized Caller**: can read when the contract is in gated-read mode.

All privileged calls — `transfer_ownership`, `set_admins`, `set_products`, `set_price_reporters`, `set_authorized_callers`, `set_paused`, `set_open_read_status` — are payable and require exactly 1 yoctoNEAR attached deposit: dApps can only request function-call access keys, which carry no allowance and cannot attach deposits, so these methods can only be invoked through a full-access key.

## Building a consumer contract

NEAR cross-contract calls are asynchronous: a consumer dispatches a Promise to the oracle and reads the result in a callback. Declare the callback argument with `#[callback_unwrap] #[serializer(borsh)]` and decode it into a mirrored struct; if the consumer re-exports prices over its own JSON ABI, wrap the price in `U128` so JS clients read a decimal string instead of a lossy number.

See [`examples/consumer`](https://github.com/oracle-atlas/near-multi-feed/tree/main/examples/consumer) for a complete, runnable reference implementation.

## Development

Prerequisites:

- [Rust](https://rustup.rs) — version pinned by `rust-toolchain.toml`.
- [`cargo-near`](https://github.com/near/cargo-near) — builds the WASM artifact.
- [`just`](https://github.com/casey/just) — the project task runner.

### Build

```bash
just build           # fast, non-reproducible WASM -> target/near/multi_feed.wasm
just build-release   # reproducible WASM (via Docker)
just check           # type-check only (fast)
```

### Test

```bash
just test            # unit tests + sandbox integration test
just test-consumer   # cross-contract end-to-end test (consumer example)
```

### Size

```bash
just size            # rebuild and print the WASM size in bytes
just size-only       # print the size without rebuilding
```

### Clean

```bash
just clean
```
