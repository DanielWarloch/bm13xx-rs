//! BM13xx ASIC representation.

#![no_std]
#![macro_use]
pub(crate) mod fmt;

pub mod core_register;
mod error;
pub mod pll;
pub mod register;
pub mod sha;

pub use self::error::{Error, Result};

use bm13xx_protocol::command::Destination;

use fugit::HertzU64;

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CmdDelay {
    pub cmd: [u8; 11],
    pub delay_ms: u32,
}

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum SequenceStep {
    #[default]
    None,
    Init(usize),
    Baudrate(usize),
    ResetCore(usize),
    HashFreq(usize),
    SplitNonce(usize),
    VersionRolling(usize),
}

/// # Hash frequency ramp settings
///
/// Used by [`Asic::set_hash_freq_ramp_next`] to move the hashing PLL towards a
/// target frequency in bounded steps, in either direction, instead of a single
/// jump (a large frequency drop is a fast load step for the core regulator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FreqRamp {
    /// Largest frequency change per step. Zero means a single jump to the target.
    pub step: HertzU64,
    /// Delay after each step, in milliseconds.
    pub step_delay_ms: u32,
}

impl FreqRamp {
    /// 6.25 MHz, the step used by the legacy BM1366/BM1370 up-ramp.
    pub const DEFAULT_STEP: HertzU64 = HertzU64::kHz(6250);
    /// 100 ms between steps.
    pub const DEFAULT_STEP_DELAY_MS: u32 = 100;

    pub const fn new(step: HertzU64, step_delay_ms: u32) -> Self {
        Self {
            step,
            step_delay_ms,
        }
    }

    /// ## Next nominal frequency of the ramp
    ///
    /// Moves `current` by at most `step` towards `target` (never past it).
    ///
    /// ### Example
    /// ```
    /// use bm13xx_asic::FreqRamp;
    /// use fugit::HertzU64;
    ///
    /// let ramp = FreqRamp::default();
    /// assert_eq!(ramp.next_freq(HertzU64::MHz(500), HertzU64::MHz(510)), HertzU64::kHz(506_250));
    /// assert_eq!(ramp.next_freq(HertzU64::kHz(506_250), HertzU64::MHz(510)), HertzU64::MHz(510));
    /// assert_eq!(ramp.next_freq(HertzU64::MHz(500), HertzU64::MHz(490)), HertzU64::kHz(493_750));
    /// assert_eq!(ramp.next_freq(HertzU64::kHz(493_750), HertzU64::MHz(490)), HertzU64::MHz(490));
    /// assert_eq!(ramp.next_freq(HertzU64::MHz(490), HertzU64::MHz(490)), HertzU64::MHz(490));
    /// // a zero step jumps straight to the target
    /// let jump = FreqRamp::new(HertzU64::Hz(0), 0);
    /// assert_eq!(jump.next_freq(HertzU64::MHz(525), HertzU64::MHz(100)), HertzU64::MHz(100));
    /// ```
    pub fn next_freq(&self, current: HertzU64, target: HertzU64) -> HertzU64 {
        if self.step.raw() == 0 {
            target
        } else if current < target {
            if target - current > self.step {
                current + self.step
            } else {
                target
            }
        } else if current - target > self.step {
            current - self.step
        } else {
            target
        }
    }
}

impl Default for FreqRamp {
    /// 6.25 MHz steps, 100 ms apart.
    fn default() -> Self {
        Self::new(Self::DEFAULT_STEP, Self::DEFAULT_STEP_DELAY_MS)
    }
}

pub trait Asic {
    fn reset(&mut self);
    fn chip_id(&self) -> u16;
    fn core_count(&self) -> usize;
    fn core_small_core_count(&self) -> usize;
    fn small_core_count(&self) -> usize;
    fn cno_interval(&self) -> usize;
    fn cno_bits(&self) -> u32;
    fn hash_freq(&self) -> HertzU64;
    fn init_next(&mut self, difficulty: u32) -> Option<CmdDelay>;
    fn set_baudrate_next(
        &mut self,
        baudrate: u32,
        chain_domain_cnt: usize,
        domain_asic_cnt: usize,
        asic_addr_interval: usize,
    ) -> Option<CmdDelay>;
    fn reset_core_next(&mut self, dest: Destination) -> Option<CmdDelay>;
    fn set_hash_freq_next(&mut self, target_freq: HertzU64) -> Option<CmdDelay>;

    /// ## Ramped Hash Frequency command list (both directions)
    ///
    /// Call repeatedly until it returns `None`; each returned command moves the
    /// hashing PLL by at most `ramp.step` towards `target_freq` and carries
    /// `ramp.step_delay_ms`. Unlike [`Asic::set_hash_freq_next`], frequency
    /// decreases are ramped too.
    ///
    /// The default implementation ignores `ramp` and falls back to
    /// [`Asic::set_hash_freq_next`], for ASICs that do not implement ramping.
    fn set_hash_freq_ramp_next(
        &mut self,
        target_freq: HertzU64,
        ramp: FreqRamp,
    ) -> Option<CmdDelay> {
        let _ = ramp;
        self.set_hash_freq_next(target_freq)
    }
    fn split_nonce_between_chips_next(
        &mut self,
        chain_asic_num: usize,
        asic_addr_interval: usize,
    ) -> Option<CmdDelay>;
    fn set_version_rolling_next(&mut self, mask: u32) -> Option<CmdDelay>;

    /// Compute the PLL register writes that set the hash frequency of a single chip
    /// (addressed by `dest`), without disturbing the chain-nominal frequency state.
    ///
    /// Returns the `(divider, parameter)` write commands, or `None` for ASICs that do
    /// not support per-chip frequency. The default is `None`; concrete ASICs override.
    fn set_hash_freq_chip_cmd(
        &self,
        dest: Destination,
        target_freq: HertzU64,
    ) -> Option<(CmdDelay, CmdDelay)> {
        let _ = (dest, target_freq);
        None
    }
}
