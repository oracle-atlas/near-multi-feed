use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::{AccountId, near};

// Pack feed data as 22 raw bytes: price (10B) + agg_ts (6B) + onchain_ts (6B).
// Custom Borsh stores exactly 22 bytes (no length prefix, no overhead);
// JSON view exposes the decoded fields.
#[near(serializers = [json])]
#[derive(Clone)]
pub struct FeedData {
    pub price: u128,
    pub agg_ts: u64,
    pub onchain_ts: u64,
}

// Custom Borsh: pack as [u8; 22] — price(10B) + agg_ts(6B) + onchain_ts(6B).

impl BorshSerialize for FeedData {
    fn serialize<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        // Write price: u128 → 16B big-endian, emit low 10 bytes.
        writer.write_all(&self.price.to_be_bytes()[6..16])?;
        // Write agg_ts: u64 → 8B big-endian, emit low 6 bytes.
        writer.write_all(&self.agg_ts.to_be_bytes()[2..8])?;
        // Write onchain_ts: u64 → 8B big-endian, emit low 6 bytes.
        writer.write_all(&self.onchain_ts.to_be_bytes()[2..8])?;
        Ok(())
    }
}

impl BorshDeserialize for FeedData {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // Read price: 10 bytes into low portion of a zeroed [u8; 16].
        let mut price_buf = [0u8; 16];
        reader.read_exact(&mut price_buf[6..16])?;
        let price = u128::from_be_bytes(price_buf);
        // Read agg_ts: 6 bytes into low portion of a zeroed [u8; 8].
        let mut agg_ts_buf = [0u8; 8];
        reader.read_exact(&mut agg_ts_buf[2..8])?;
        let agg_ts = u64::from_be_bytes(agg_ts_buf);
        // Read onchain_ts: 6 bytes into low portion of a zeroed [u8; 8].
        let mut onchain_ts_buf = [0u8; 8];
        reader.read_exact(&mut onchain_ts_buf[2..8])?;
        let onchain_ts = u64::from_be_bytes(onchain_ts_buf);
        Ok(Self {
            price,
            agg_ts,
            onchain_ts,
        })
    }
}

// Flags: bit-packed u8 — openRead (bit 0) + paused (bit 1).

// Map bit 0 to openRead, bit 1 to paused.
const OPEN_READ_MASK: u8 = 1 << 0;
const PAUSED_MASK: u8 = 1 << 1;

// Serialize flags as a single u8 via Borsh.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, borsh::BorshSchema)]
pub struct Flags(u8);

impl Flags {
    // Construct flags from initial paused and open_read states.
    pub fn new(is_paused: bool, is_open_read: bool) -> Self {
        Flags(is_open_read as u8 | (is_paused as u8) << 1)
    }

    // Return whether the contract is paused (flags > 1).
    pub fn is_paused(&self) -> bool {
        self.0 > 1
    }

    // Return whether open-read is set AND the contract is not paused (flags == 1).
    pub fn is_open_read_when_unpaused(&self) -> bool {
        self.0 == OPEN_READ_MASK
    }

    // Return whether the open-read flag is set.
    pub fn is_open_read(&self) -> bool {
        self.0 & OPEN_READ_MASK != 0
    }

    // Toggle the paused flag.
    pub fn flip_paused(&mut self) {
        self.0 ^= PAUSED_MASK;
    }

    // Toggle the open-read flag.
    pub fn flip_open_read(&mut self) {
        self.0 ^= OPEN_READ_MASK;
    }
}

// Submit a single price feed entry via a reporter.
// Borsh-only: `f` is a high-frequency backend-only batch entrypoint;
// borsh decoding is far cheaper than JSON parsing — the biggest input-side gas saving.
#[near(serializers = [borsh])]
pub struct FeedUpdate {
    pub feed_id: u32,
    pub price: u128,
    pub agg_ts: u64,
}

// Update role membership: account + desired status (add = true, remove = false).
#[near(serializers = [json])]
pub struct RoleUpdate {
    pub account: AccountId,
    pub status: bool,
}
