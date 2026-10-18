//! On-die hash counters of the BM1366 / BM1368 / BM1370 family.
//!
//! These registers are free-running `u32` counters, where each count stands for
//! [`HASHES_PER_COUNT`] (2^32) hashes. Read them periodically and compute the
//! delta (modulo 2^32, see [`TotalHashCounter::hashes_since`]) to get an on-chip
//! hashrate that does not depend on ticket-mask luck.
//!
//! **Provenance:** the addresses and the 2^32 hashes/count unit come from
//! ESP-Miner's usage (its hashrate monitor reads `0x8C` and `0x88`–`0x8B` on every
//! chip), not from Bitmain documentation. They are unverified by Bitmain docs.

use crate::register::Register;

/// Number of hashes represented by one count of a hash counter register
/// (per ESP-Miner, unverified by Bitmain docs).
pub const HASHES_PER_COUNT: u64 = 1 << 32;

macro_rules! hash_counter {
    ($(#[$meta:meta])* $REG:ident, $ADDR:expr) => {
        $(#[$meta])*
        #[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
        pub struct $REG(pub u32);
        impl_boilerplate_for!($REG);

        impl $REG {
            pub const ADDR: u8 = $ADDR;

            /// ## Raw counter value.
            ///
            /// Each count stands for [`HASHES_PER_COUNT`] hashes.
            pub const fn count(&self) -> u32 {
                self.0
            }

            /// ## Hashes done since an `earlier` reading of the same register.
            ///
            /// Handles a single `u32` wrap-around between the two readings
            /// (wrapping subtraction), then scales by [`HASHES_PER_COUNT`].
            pub const fn hashes_since(&self, earlier: Self) -> u64 {
                (self.0.wrapping_sub(earlier.0) as u64) * HASHES_PER_COUNT
            }
        }

        impl core::fmt::Display for $REG {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_struct(stringify!($REG))
                    .field("count", &self.count())
                    .finish()
            }
        }

        #[cfg(feature = "defmt")]
        impl defmt::Format for $REG {
            fn format(&self, fmt: defmt::Formatter) {
                defmt::write!(fmt, "{} {{ count: {} }}", stringify!($REG), self.count());
            }
        }
    };
}

hash_counter!(
    /// # Total Hash Counter register (0x8C)
    ///
    /// Free-running count of all hashes done by the chip, one count per 2^32
    /// hashes. Per ESP-Miner, unverified by Bitmain docs. Formerly exposed by
    /// this crate as `ClockOrderStatus` (the BM1397-era name for `0x8C`).
    ///
    /// ### Example
    ///
    /// ```
    /// use bm13xx_asic::register::{TotalHashCounter, HASHES_PER_COUNT};
    ///
    /// let t0 = TotalHashCounter(0xffff_fffe);
    /// let t1 = TotalHashCounter(0x0000_0003); // wrapped around
    /// assert_eq!(TotalHashCounter::ADDR, 0x8C);
    /// assert_eq!(t1.count(), 3);
    /// assert_eq!(t1.hashes_since(t0), 5 * HASHES_PER_COUNT);
    /// ```
    TotalHashCounter,
    0x8C
);

hash_counter!(
    /// # Hash Domain 0 Counter register (0x88)
    ///
    /// Free-running count of hashes done by hash domain 0, one count per 2^32
    /// hashes. Per ESP-Miner, unverified by Bitmain docs.
    ///
    /// ### Example
    ///
    /// ```
    /// use bm13xx_asic::register::{DomainHashCounter0, HASHES_PER_COUNT};
    ///
    /// assert_eq!(DomainHashCounter0::ADDR, 0x88);
    /// assert_eq!(DomainHashCounter0(12).hashes_since(DomainHashCounter0(10)), 2 * HASHES_PER_COUNT);
    /// ```
    DomainHashCounter0,
    0x88
);

hash_counter!(
    /// # Hash Domain 1 Counter register (0x89)
    ///
    /// Free-running count of hashes done by hash domain 1, one count per 2^32
    /// hashes. Note the unaligned address. Per ESP-Miner, unverified by Bitmain docs.
    DomainHashCounter1,
    0x89
);

hash_counter!(
    /// # Hash Domain 2 Counter register (0x8A)
    ///
    /// Free-running count of hashes done by hash domain 2, one count per 2^32
    /// hashes. Note the unaligned address. Per ESP-Miner, unverified by Bitmain docs.
    DomainHashCounter2,
    0x8A
);

hash_counter!(
    /// # Hash Domain 3 Counter register (0x8B)
    ///
    /// Free-running count of hashes done by hash domain 3, one count per 2^32
    /// hashes. Note the unaligned address. Per ESP-Miner, unverified by Bitmain docs.
    DomainHashCounter3,
    0x8B
);

/// Addresses of the per-domain hash counters, indexed by domain
/// (per ESP-Miner, unverified by Bitmain docs).
///
/// ```
/// use bm13xx_asic::register::{DomainHashCounter3, DOMAIN_HASH_COUNTER_ADDRS};
///
/// assert_eq!(DOMAIN_HASH_COUNTER_ADDRS, [0x88, 0x89, 0x8A, 0x8B]);
/// assert_eq!(DOMAIN_HASH_COUNTER_ADDRS[3], DomainHashCounter3::ADDR);
/// ```
pub const DOMAIN_HASH_COUNTER_ADDRS: [u8; 4] = [
    DomainHashCounter0::ADDR,
    DomainHashCounter1::ADDR,
    DomainHashCounter2::ADDR,
    DomainHashCounter3::ADDR,
];

/// Deprecated name of [`TotalHashCounter`] (register `0x8C`).
///
/// Only the associated items (e.g. `ClockOrderStatus::ADDR`) are usable through
/// this alias; construct values with `TotalHashCounter(..)`.
#[deprecated(
    note = "0x8C is the total hash counter on BM1366/BM1368/BM1370; use `TotalHashCounter`"
)]
pub type ClockOrderStatus = TotalHashCounter;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_since_no_wrap() {
        assert_eq!(TotalHashCounter(7).hashes_since(TotalHashCounter(7)), 0);
        assert_eq!(
            TotalHashCounter(100).hashes_since(TotalHashCounter(1)),
            99 * HASHES_PER_COUNT
        );
    }

    #[test]
    fn hashes_since_wraps() {
        assert_eq!(
            DomainHashCounter2(0).hashes_since(DomainHashCounter2(u32::MAX)),
            HASHES_PER_COUNT
        );
    }

    #[test]
    #[allow(deprecated)]
    fn deprecated_alias_keeps_address() {
        assert_eq!(ClockOrderStatus::ADDR, 0x8C);
        assert_eq!(TotalHashCounter(1).addr(), 0x8C);
    }
}
