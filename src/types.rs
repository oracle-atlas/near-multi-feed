use near_sdk::{
    AccountId,
    borsh::{BorshDeserialize, BorshSerialize},
    near,
};

// Internal storage format: pack a feed into 22 raw bytes — price (10B) +
// agg_ts (6B) + onchain_ts (6B), big-endian with no length prefix or
// padding, 10 bytes below the standard full-width encoding. Minimizing
// stored bytes directly minimizes the NEAR locked by per-byte storage
// staking. Not part of the contract ABI.
pub struct StoredFeed {
    pub price: u128,
    pub agg_ts: u64,
    pub onchain_ts: u64,
}

impl BorshSerialize for StoredFeed {
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

impl BorshDeserialize for StoredFeed {
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

// Returned by `fetch`/`fetch_batch`, encoded as standard Borsh
// (little-endian, full-width fields), so consumers decode with any
// Borsh library rather than the custom 22-byte storage codec.
#[near(serializers = [borsh])]
pub struct FeedData {
    pub price: u128,
    pub agg_ts: u64,
    pub onchain_ts: u64,
}

impl From<&StoredFeed> for FeedData {
    fn from(sf: &StoredFeed) -> Self {
        Self {
            price: sf.price,
            agg_ts: sf.agg_ts,
            onchain_ts: sf.onchain_ts,
        }
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

    mod stored_feed {
        use crate::types::StoredFeed;
        use near_sdk::borsh::{self, BorshDeserialize};

        #[test]
        fn round_trips_22_byte_codec() {
            // Verify the custom storage codec round-trips the field-wise maxima;
            // price is truncated to 10B and timestamps to 6B, both big-endian.
            let stored = StoredFeed {
                price: (1u128 << 80) - 1,
                agg_ts: (1u64 << 48) - 1,
                onchain_ts: (1u64 << 48) - 1,
            };
            let bytes = borsh::to_vec(&stored).unwrap();
            assert_eq!(bytes.len(), 22);
            let decoded = StoredFeed::try_from_slice(&bytes).unwrap();
            assert_eq!(decoded.price, stored.price);
            assert_eq!(decoded.agg_ts, stored.agg_ts);
            assert_eq!(decoded.onchain_ts, stored.onchain_ts);
        }

        #[test]
        fn byte_layout_is_pinned() {
            // Pin the exact 22-byte storage layout (price, then agg_ts,
            // then onchain_ts, each big-endian) so accidental format drift
            // breaks loudly — stored state must stay decodable across
            // contract upgrades.
            let stored = StoredFeed {
                price: 0x0102030405060708090A,
                agg_ts: 0x010203040506,
                onchain_ts: 0x010203040506,
            };
            let bytes = borsh::to_vec(&stored).unwrap();
            let expected: [u8; 22] = [
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x01, 0x02, 0x03, 0x04,
                0x05, 0x06, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            ];
            assert_eq!(bytes, expected);
        }

        #[test]
        fn drops_price_bits_above_80() {
            // Document the codec's truncation semantics: only the low 80
            // price bits survive a round trip — inputs at or above 2^80 are
            // silently corrupted unless bounded upstream.
            let stored = StoredFeed {
                price: (1u128 << 80) + 0x1234,
                agg_ts: 0,
                onchain_ts: 0,
            };
            let bytes = borsh::to_vec(&stored).unwrap();
            let decoded = StoredFeed::try_from_slice(&bytes).unwrap();
            assert_eq!(decoded.price, 0x1234);
        }
    }
}
