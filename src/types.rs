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

// Bit-packed contract flags: openRead (bit 0) + paused (bit 1).
// Borsh serializes as a single u8.
#[derive(borsh::BorshSerialize, borsh::BorshDeserialize, borsh::BorshSchema)]
pub struct Flags(u8);

impl Flags {
    const OPEN_READ_MASK: u8 = 1 << 0;
    const PAUSED_MASK: u8 = 1 << 1;

    // Construct flags from initial paused and open_read states.
    pub fn new(is_paused: bool, is_open_read: bool) -> Self {
        Flags(is_open_read as u8 | (is_paused as u8) << 1)
    }

    // Return whether the contract is paused.
    pub fn is_paused(&self) -> bool {
        //  cheapest single comparison.
        self.0 > 1
    }

    // Return whether open-read is set AND the contract is not paused.
    pub fn is_open_read_when_unpaused(&self) -> bool {
        self.0 == Self::OPEN_READ_MASK
    }

    // Return whether the open-read flag is set.
    pub fn is_open_read(&self) -> bool {
        self.0 & Self::OPEN_READ_MASK != 0
    }

    // Toggle the paused flag.
    pub fn flip_paused(&mut self) {
        self.0 ^= Self::PAUSED_MASK;
    }

    // Toggle the open-read flag.
    pub fn flip_open_read(&mut self) {
        self.0 ^= Self::OPEN_READ_MASK;
    }
}

// Submit a single price feed entry.
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

#[cfg(test)]
mod tests {
    mod flags {
        use crate::types::Flags;

        mod new {
            use super::*;

            #[test]
            fn neither_paused_nor_open_read() {
                let f = Flags::new(false, false);
                assert_eq!(f.0, 0);
                assert!(!f.is_paused());
                assert!(!f.is_open_read());
                assert!(!f.is_open_read_when_unpaused());
            }

            #[test]
            fn open_read_only() {
                let f = Flags::new(false, true);
                assert_eq!(f.0, 1);
                assert!(!f.is_paused());
                assert!(f.is_open_read());
                assert!(f.is_open_read_when_unpaused());
            }

            #[test]
            fn paused_only() {
                let f = Flags::new(true, false);
                assert_eq!(f.0, 2);
                assert!(f.is_paused());
                assert!(!f.is_open_read());
                assert!(!f.is_open_read_when_unpaused());
            }

            #[test]
            fn both_paused_and_open_read() {
                let f = Flags::new(true, true);
                assert_eq!(f.0, 3);
                assert!(f.is_paused());
                assert!(f.is_open_read());
                assert!(!f.is_open_read_when_unpaused());
            }
        }

        mod is_paused {
            use super::*;

            #[test]
            fn flags_0() {
                assert!(!Flags::new(false, false).is_paused());
            }

            #[test]
            fn flags_1() {
                assert!(!Flags::new(false, true).is_paused());
            }

            #[test]
            fn flags_2() {
                assert!(Flags::new(true, false).is_paused());
            }

            #[test]
            fn flags_3() {
                assert!(Flags::new(true, true).is_paused());
            }
        }

        mod is_open_read_when_unpaused {
            use super::*;

            #[test]
            fn flags_0() {
                assert!(!Flags::new(false, false).is_open_read_when_unpaused());
            }

            #[test]
            fn flags_1() {
                assert!(Flags::new(false, true).is_open_read_when_unpaused());
            }

            #[test]
            fn flags_2() {
                assert!(!Flags::new(true, false).is_open_read_when_unpaused());
            }

            #[test]
            fn flags_3() {
                assert!(!Flags::new(true, true).is_open_read_when_unpaused());
            }
        }

        mod is_open_read {
            use super::*;

            #[test]
            fn not_set() {
                assert!(!Flags::new(false, false).is_open_read());
            }

            #[test]
            fn not_set_when_paused() {
                assert!(!Flags::new(true, false).is_open_read());
            }

            #[test]
            fn set_without_paused() {
                assert!(Flags::new(false, true).is_open_read());
            }

            #[test]
            fn set_with_paused() {
                assert!(Flags::new(true, true).is_open_read());
            }
        }

        mod flip_paused {
            use super::*;

            #[test]
            fn from_unpaused() {
                let mut f = Flags::new(false, false);
                f.flip_paused();
                assert!(f.is_paused());
                assert_eq!(f.0, 2);
            }

            #[test]
            fn from_paused() {
                let mut f = Flags::new(true, false);
                f.flip_paused();
                assert!(!f.is_paused());
                assert_eq!(f.0, 0);
            }

            #[test]
            fn with_open_read() {
                let mut f = Flags::new(false, true);
                f.flip_paused();
                assert!(f.is_paused());
                assert!(f.is_open_read());
                assert_eq!(f.0, 3);

                f.flip_paused();
                assert!(!f.is_paused());
                assert!(f.is_open_read());
                assert_eq!(f.0, 1);
            }
        }

        mod flip_open_read {
            use super::*;

            #[test]
            fn from_unset() {
                let mut f = Flags::new(false, false);
                f.flip_open_read();
                assert!(f.is_open_read());
                assert_eq!(f.0, 1);
            }

            #[test]
            fn from_set() {
                let mut f = Flags::new(false, true);
                f.flip_open_read();
                assert!(!f.is_open_read());
                assert_eq!(f.0, 0);
            }

            #[test]
            fn with_paused() {
                let mut f = Flags::new(true, false);
                f.flip_open_read();
                assert!(f.is_open_read());
                assert!(f.is_paused());
                assert_eq!(f.0, 3);

                f.flip_open_read();
                assert!(!f.is_open_read());
                assert!(f.is_paused());
                assert_eq!(f.0, 2);
            }
        }
    }
}
