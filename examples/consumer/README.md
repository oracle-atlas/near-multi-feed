# Multi Feed Consumer Example

Demonstrates how to read prices from a deployed `multi-feed` oracle via cross-contract calls. The consumer uses `#[ext_contract]` to define the oracle interface and forwards results through `#[private]` callbacks.

Not audited — for integration reference and end-to-end testing only.

## Build

```bash
just build-consumer
```

## Test

```bash
just test-consumer
```

## Key Files

- `src/ext.rs` — self-contained cross-contract interface (`FeedData` + `#[ext_contract]` trait). Copy this file into your own project to integrate with multi-feed.
- `src/lib.rs` — consumer contract logic (cross-contract call + callback).
