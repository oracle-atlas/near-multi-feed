# Multi-Feed — task runner
# Run `just` (or `just --list`) to see available recipes.

# Path to the deployable wasm produced by `cargo near build`.
wasm := "target/near/multi_feed.wasm"

# Type-check the contract for the wasm target (fast, no artifact).
check:
    cargo check --target wasm32-unknown-unknown

# Run all tests (unit + sandbox integration).
test:
    cargo test

# Development build (fast, non-reproducible). Produces {{wasm}}.
build:
    cargo near build non-reproducible-wasm

# Production build (reproducible, via Docker; requires committed git state).
build-release:
    cargo near build reproducible-wasm

# Show the deployable wasm size in bytes (this is the on-chain contract size).
# Runs a fresh dev build first so the number reflects current code.
size: build
    @echo "{{wasm}}: $(stat -f%z {{wasm}}) bytes"

# Show the wasm size without rebuilding (uses the existing artifact).
size-only:
    @test -f {{wasm}} || (echo "No wasm found — run 'just build' first" && exit 1)
    @echo "{{wasm}}: $(stat -f%z {{wasm}}) bytes"

# Remove build artifacts.
clean:
    cargo clean

# Run the consumer example's integration test (sandbox).
test-consumer:
    cd examples/consumer && cargo test --test integration

# Build the consumer example contract.
build-consumer:
    cd examples/consumer && cargo near build non-reproducible-wasm

# Deploy the contract to a client (from deployments.toml).
# Onboarding a new client:
#   1. near account create-account fund-myself <client>.atlas-oracle.near autogenerate-new-keypair --accountId atlas-oracle.near
#   2. Add [<client>] section to deployments.toml
#   3. just build-release
#   4. just deploy <client>
# Usage: just deploy <client>
deploy client:
    cd scripts && cargo run --bin deploy -- --client {{client}}

# Submit price feed data for a client.
# Usage: just feed <account> <count>
# Example: just feed multi-feed-qa-1.atlas-oracle.near 5
feed account count:
    cd scripts && cargo run --bin feed -- --account {{account}} --count {{count}}
