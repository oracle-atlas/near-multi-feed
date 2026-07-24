# Multi Feed (NEAR)

A NEAR price feed oracle contract. Backend reporters submit batched price data via Borsh; consumers read via JSON. Feed IDs follow the EVM `bytes4` convention (`0x` + 8 hex chars).

## Build

```bash
# dev build → target/near/multi_feed.wasm
just build
# reproducible build (Docker)
just build-release
# build + show wasm size
just size
# type-check only (fast)
just check
```

## Test

```bash
# unit + sandbox integration
just test
# unit tests only
cargo test --lib
```

## Read Interface

| Method | Returns | Description |
|--------|---------|-------------|
| `fetch(feed_id)` | `Option<FeedData>` | Single feed by ID — `None` if absent |
| `fetch_batch(feed_ids)` | `Vec<Option<FeedData>>` | Batch feed lookup — each element is `None` if absent |
| `get_owner()` | `AccountId` | Contract owner |
| `description()` | `String` | Human-readable contract description |
| `decimals()` | `u8` | Price decimals |
| `version()` | `u8` | Contract version |
| `is_admin(account)` | `bool` | Check admin role |
| `is_product(account)` | `bool` | Check product role |
| `is_price_reporter(account)` | `bool` | Check price reporter role |
| `is_authorized_caller(account)` | `bool` | Check authorized caller |
| `is_paused()` | `bool` | Whether contract is paused |
| `is_open_read()` | `bool` | Whether open-read is enabled |
| `get_authorized_callers()` | `Vec<AccountId>` | All authorized callers |

## Read Modes

Two read modes, controlled by the owner:

- **Open read** (`is_open_read() == true`) — anyone can call `fetch` / `fetch_batch`. Set at init with `open_read_enabled: true`.
- **Gated read** (`is_open_read() == false`) — only addresses in the authorized-caller whitelist can read. The whitelist is managed by products.

Either mode is blocked when the contract is paused.

## Feed Data

What `fetch` and `fetch_batch` return. Each entry is the latest price for a feed ID.

```json
{
  "price": "123456789012345678",
  "agg_ts": 1700000000,
  "onchain_ts": 1700000001
}
```

| Field | Type | Description |
|-------|------|-------------|
| `price` | `u128` | Raw price value (18 decimal places) |
| `agg_ts` | `u64` | Reporter's aggregated timestamp (seconds), strictly monotonic per feed |
| `onchain_ts` | `u64` | Block timestamp at write time (seconds) |

## Borsh Input for `f`

`f` is the batch feed entrypoint. It takes Borsh-encoded `Vec<FeedUpdate>` instead of JSON — Borsh costs far less gas to decode, and this gets called a lot.

| Field | Type | Size |
|-------|------|------|
| `feed_id` | `u32` | 4 bytes |
| `price` | `u128` | 16 bytes |
| `agg_ts` | `u64` | 8 bytes |

## Feed ID Format

`0x` + 8 case-insensitive hex digits. Example: `"0x0000002a"`.

## Storage

To minimize on-chain storage gas, `FeedData` uses a custom Borsh serialization that packs each entry into 22 bytes instead of the native 36 bytes:

| Field | Native | Packed | Truncation |
|-------|--------|--------|------------|
| `price` | 16 bytes | 10 bytes | Low 80 bits (covers up to ~1.2e24) |
| `agg_ts` | 8 bytes | 6 bytes | Low 48 bits (covers ~8.9M years) |
| `onchain_ts` | 8 bytes | 6 bytes | Low 48 bits |

The discarded bytes are never needed — the remaining bits can already represent prices up to ~1.2e24 and timestamps spanning ~8.9 million years.

## Access Control

- **Owner**: full control — transfer ownership, manage roles, toggle open-read.
- **Admin**: can pause / unpause.
- **Product**: can manage the authorized-caller whitelist.
- **Price Reporter**: can submit feeds via `f`.
- **Authorized Caller**: can read when the contract is in gated-read mode.