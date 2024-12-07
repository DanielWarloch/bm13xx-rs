//! BM1370 ASIC implementation.

#![no_std]
#![macro_use]
pub(crate) mod fmt;

use bm13xx_asic::{core_register::*, register::*, Asic, CmdDelay};
use bm13xx_protocol::command::{Command, Destination};

use core::time::Duration;
use fugit::HertzU64;
use heapless::{FnvIndexMap, Vec};

pub const BM1370_CHIP_ID: u16 = 0x1370;
pub const BM1370_CORE_CNT: usize = 128;
pub const BM1370_SMALL_CORE_CNT: usize = 2040;
pub const BM1370_CORE_SMALL_CORE_CNT: usize = 16;
pub const BM1370_DOMAIN_CNT: usize = 4;
pub const BM1370_PLL_CNT: usize = 4;
pub const BM1370_PLL_ID_HASH: usize = 0; // PLL0 isused for Hashing
pub const BM1370_PLL_OUT_HASH: usize = 0; // specifically PLL0_OUT0 can be used for Hashing
pub const BM1370_PLL_ID_UART: usize = 3; // PLL3 can be used for UART Baudrate
pub const BM1370_PLL_OUT_UART: usize = 4; // specifically PLL1_OUT4 can be used for UART Baudrate
pub const BM1370_NONCE_CORES_BITS: usize = 7; // TODO: Check if is correct
pub const BM1370_NONCE_CORES_MASK: u32 = 0b111_1111; // TODO: Check if is correct
pub const BM1370_NONCE_SMALL_CORES_BITS: usize = 3; // TODO: Check if is correct
pub const BM1370_NONCE_SMALL_CORES_MASK: u32 = 0b111; // TODO: Check if is correct

const NONCE_BITS: usize = 32;
const CHIP_ADDR_BITS: usize = 8;
const CHIP_ADDR_MASK: u32 = 0b1111_1111;

// TODO: Check and correct values in all of the Examples

/// # BM1370
#[derive(Debug)]
// #[cfg_attr(feature = "defmt-03", derive(defmt::Format))] // FnvIndexMap doesn't implement defmt
pub struct BM1370 {
    pub sha: bm13xx_asic::sha::Sha<
        BM1370_CORE_CNT,
        BM1370_SMALL_CORE_CNT,
        BM1370_CORE_SMALL_CORE_CNT,
        BM1370_DOMAIN_CNT,
    >,
    pub input_clock_freq: HertzU64,
    pub plls: [bm13xx_asic::pll::Pll; BM1370_PLL_CNT],
    pub chip_addr: u8,
    pub registers: FnvIndexMap<u8, u32, 64>,
    pub core_registers: FnvIndexMap<u8, u8, 16>,
    pub version_rolling_enabled: bool,
    pub version_mask: u32,
}

impl BM1370 {
    pub fn new_with_clk(clk: HertzU64) -> Self {
        BM1370 {
            input_clock_freq: clk,
            ..Default::default()
        }
    }

    /// ## Set the Chip Address
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let mut bm1370 = BM1370::default();
    /// bm1370.set_chip_addr(2);
    /// assert_eq!(bm1370.chip_addr, 2);
    /// ```
    pub fn set_chip_addr(&mut self, chip_addr: u8) {
        self.chip_addr = chip_addr;
    }

    /// ## Enable the Hardware Version Rolling
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let mut bm1370 = BM1370::default();
    /// bm1370.enable_version_rolling(0x1fffe000);
    /// assert!(bm1370.version_rolling_enabled);
    /// assert_eq!(bm1370.version_mask, 0x1fffe000);
    /// ```
    pub fn enable_version_rolling(&mut self, version_mask: u32) {
        self.version_rolling_enabled = true;
        self.version_mask = version_mask;
    }

    fn version_mask_bits(&self) -> usize {
        self.version_mask.count_ones() as usize
    }

    /// ## Get the SHA Hashing Frequency
    ///
    /// ### Example
    /// ```
    /// use bm1370::{BM1370, BM1370_PLL_ID_HASH};
    /// use fugit::HertzU64;
    ///
    /// let mut bm1370 = BM1370::default();
    /// assert_eq!(bm1370.hash_freq(), HertzU64::MHz(50));
    /// assert_eq!(bm1370.set_hash_freq(HertzU64::MHz(200)).hash_freq(), HertzU64::MHz(200));
    /// ```
    pub fn hash_freq(&self) -> HertzU64 {
        self.plls[BM1370_PLL_ID_HASH].frequency(self.input_clock_freq, BM1370_PLL_OUT_HASH)
    }
    pub fn set_hash_freq(&mut self, freq: HertzU64) -> &mut Self {
        self.plls[BM1370_PLL_ID_HASH].set_frequency(
            self.input_clock_freq,
            BM1370_PLL_OUT_HASH,
            freq,
        );
        self
    }

    /// ## Get the theoretical Hashrate in GH/s
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use fugit::HertzU64;
    ///
    /// let bm1370 = BM1370::default();
    /// assert_eq!(bm1370.theoretical_hashrate_ghs(), 44.7);
    /// ```
    pub fn theoretical_hashrate_ghs(&self) -> f32 {
        self.hash_freq().raw() as f32 * self.sha.small_core_count() as f32 / 1_000_000_000.0
    }

    /// ## Get the rolling duration
    ///
    /// BM1370 can do Version Rolling in Hardware.
    ///
    /// If Hardware Version Rolling is not enabled, BM1370 only roll the Nonce Space (32 bits), but:
    /// - Nonce\[31:25\] is used to hardcode the Core ID.
    /// - Nonce\[24:22\] is used to hardcode the Small Core ID.
    /// - Nonce\[21:14\] is used to hardcode the Chip Address.
    ///
    /// So only the Nonce\[13:0\] are rolled for each Chip Address.
    ///
    /// If Hardware Version Rolling is enabled, BM1370 roll the Nonce Space (32 bits) and
    /// up to 16 bits in Version Space, but:
    /// - Nonce\[31:25\] is used to hardcode the Core ID.
    /// - Nonce\[24:17\] is used to hardcode the Chip Address.
    /// - Version\[15:13\] is used to hardcode the Small Core ID (assuming the Version Mask is 0x1fffe000).
    ///
    /// So only the Nonce\[16:0\] and Version\[28:16\] are rolled for each Chip Address.
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use core::time::Duration;
    ///
    /// let mut bm1370 = BM1370::default();
    /// assert_eq!(bm1370.rolling_duration(), Duration::from_secs_f32(0.00032768));
    /// bm1370.enable_version_rolling(0x1fffe000);
    /// assert_eq!(bm1370.rolling_duration(), Duration::from_secs_f32(21.474836349));
    /// ```
    pub fn rolling_duration(&self) -> Duration {
        let space = if self.version_rolling_enabled {
            (1 << (NONCE_BITS - BM1370_NONCE_CORES_BITS - CHIP_ADDR_BITS
                + self.version_mask_bits()
                - BM1370_NONCE_SMALL_CORES_BITS)) as f32
        } else {
            (1 << (NONCE_BITS
                - BM1370_NONCE_CORES_BITS
                - BM1370_NONCE_SMALL_CORES_BITS
                - CHIP_ADDR_BITS)) as f32
        };
        Duration::from_secs_f32(space / (self.hash_freq().raw() as f32))
    }

    /// ## Get the Core ID that produced a given Nonce
    ///
    /// Core ID is always hardcoded in Nonce\[31:25\].
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let bm1370 = BM1370::default();
    /// assert_eq!(bm1370.nonce2core_id(0x12345678), 0x09);
    /// assert_eq!(bm1370.nonce2core_id(0x906732c8), 72); // first Bitaxe Block 853742
    /// ```
    pub fn nonce2core_id(&self, nonce: u32) -> usize {
        ((nonce >> (NONCE_BITS - BM1370_NONCE_CORES_BITS)) & BM1370_NONCE_CORES_MASK) as usize
    }

    /// ## Get the Small Core ID that produced a given Nonce
    ///
    /// If the Hardware Version Rolling is disabled, the Small Core ID is hardcoded in Nonce\[24:22\].
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let bm1370 = BM1370::default();
    /// assert_eq!(bm1370.nonce2small_core_id(0x12045678), 0);
    /// assert_eq!(bm1370.nonce2small_core_id(0x12445678), 1);
    /// assert_eq!(bm1370.nonce2small_core_id(0x12845678), 2);
    /// assert_eq!(bm1370.nonce2small_core_id(0x12c45678), 3);
    /// assert_eq!(bm1370.nonce2small_core_id(0x13045678), 4);
    /// assert_eq!(bm1370.nonce2small_core_id(0x13445678), 5);
    /// assert_eq!(bm1370.nonce2small_core_id(0x13845678), 6);
    /// assert_eq!(bm1370.nonce2small_core_id(0x13c45678), 7);
    /// ```
    pub fn nonce2small_core_id(&self, nonce: u32) -> usize {
        ((nonce >> (NONCE_BITS - BM1370_NONCE_CORES_BITS - BM1370_NONCE_SMALL_CORES_BITS))
            & BM1370_NONCE_SMALL_CORES_MASK) as usize
    }

    /// ## Get the Small Core ID that produced a given Version
    ///
    /// If the Hardware Version Rolling is enabled, the Small Core ID is hardcoded in Version\[15:13\]
    /// (assuming the Version Mask is 0x1fffe000).
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let mut bm1370 = BM1370::default();
    /// bm1370.enable_version_rolling(0x1fffe000);
    /// assert_eq!(bm1370.version2small_core_id(0x1fff0000), 0);
    /// assert_eq!(bm1370.version2small_core_id(0x1fff2000), 1);
    /// assert_eq!(bm1370.version2small_core_id(0x1fff4000), 2);
    /// assert_eq!(bm1370.version2small_core_id(0x1fff6000), 3);
    /// assert_eq!(bm1370.version2small_core_id(0x1fff8000), 4);
    /// assert_eq!(bm1370.version2small_core_id(0x1fffa000), 5);
    /// assert_eq!(bm1370.version2small_core_id(0x1fffd000), 6);
    /// assert_eq!(bm1370.version2small_core_id(0x1fffe000), 7);
    /// assert_eq!(bm1370.version2small_core_id(0x00f94000), 2); // first Bitaxe Block 853742
    /// ```
    pub fn version2small_core_id(&self, version: u32) -> usize {
        ((version >> self.version_mask.trailing_zeros()) & BM1370_NONCE_SMALL_CORES_MASK) as usize
    }

    /// ## Get the Chip Address that produced a given Nonce
    ///
    /// If the Hardware Version Rolling is enabled, the Chip Address is hardcoded in Nonce\[24:17\],
    /// else it is hardcoded in Nonce\[21:14\].
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    ///
    /// let mut bm1370 = BM1370::default();
    /// assert_eq!(bm1370.nonce2chip_addr(0x12345678), 0xD1);
    /// bm1370.enable_version_rolling(0x1fffe000);
    /// assert_eq!(bm1370.nonce2chip_addr(0x12345679), 0x1A);
    /// ```
    pub fn nonce2chip_addr(&self, nonce: u32) -> usize {
        if self.version_rolling_enabled {
            ((nonce >> (NONCE_BITS - BM1370_NONCE_CORES_BITS - CHIP_ADDR_BITS)) & CHIP_ADDR_MASK)
                as usize
        } else {
            ((nonce
                >> (NONCE_BITS
                    - BM1370_NONCE_CORES_BITS
                    - BM1370_NONCE_SMALL_CORES_BITS
                    - CHIP_ADDR_BITS))
                & CHIP_ADDR_MASK) as usize
        }
    }
}

impl Default for BM1370 {
    fn default() -> Self {
        let mut bm1370 = Self {
            sha: bm13xx_asic::sha::Sha::default(),
            input_clock_freq: HertzU64::MHz(25),
            plls: [bm13xx_asic::pll::Pll::default(); BM1370_PLL_CNT],
            chip_addr: 0,
            registers: FnvIndexMap::<_, _, 64>::new(),
            core_registers: FnvIndexMap::<_, _, 16>::new(),
            version_rolling_enabled: false,
            version_mask: 0x1fffe000,
        };
        // Default PLLs Parameter
        bm1370.plls[0].set_parameter(0xC054_0165);
        bm1370.plls[1].set_parameter(0x2050_0174);
        bm1370.plls[2].set_parameter(0x2050_0174);
        bm1370.plls[3].set_parameter(0x0000_0000);
        // Default PLLs Divider
        bm1370.plls[0].set_divider(0x0000_0000);
        bm1370.plls[1].set_divider(0x0000_0000);
        bm1370.plls[2].set_divider(0x0000_0000);
        bm1370.plls[3].set_divider(0x0000_0000);
        // Default Registers Value
        bm1370
            .registers
            .insert(ChipIdentification::ADDR, 0x1370_0000)
            .unwrap();
        bm1370
            .registers
            .insert(HashRate::ADDR, 0x0001_2a89)
            .unwrap();
        bm1370
            .registers
            .insert(PLL0Parameter::ADDR, 0xc054_0165)
            .unwrap();
        bm1370
            .registers
            .insert(ChipNonceOffsetV2::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(HashCountingNumber::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(TicketMask::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(MiscControl::ADDR, 0x0000_c100)
            .unwrap();
        bm1370
            .registers
            .insert(I2CControl::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            // .insert(OrderedClockEnable::ADDR, 0x0000_0003) // NOTE: Changed from 1360
            .insert(OrderedClockEnable::ADDR, 0x0000_0007)
            .unwrap();
        bm1370.registers.insert(Reg24::ADDR, 0x0010_0000).unwrap();
        bm1370
            .registers
            .insert(FastUARTConfigurationV2::ADDR, 0x0130_1a00)
            .unwrap();
        bm1370
            .registers
            .insert(UARTRelay::ADDR, 0x000f_0000)
            .unwrap();
        bm1370.registers.insert(Reg30::ADDR, 0x0000_0080).unwrap();
        // bm1370.registers.insert(Reg30::ADDR, 0x0000_0070).unwrap(); // NOTE: changed from 1360
        bm1370.registers.insert(Reg34::ADDR, 0x0000_0000).unwrap();
        bm1370
            .registers
            .insert(TicketMask2::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(CoreRegisterControl::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(CoreRegisterValue::ADDR, 0x007f_0000)
            // .insert(CoreRegisterValue::ADDR, 0x1eaf_5fbe) // NOTE: changed from 1360
            .unwrap();
        bm1370
            .registers
            .insert(ExternalTemperatureSensorRead::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(ErrorFlag::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(NonceErrorCounter::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(NonceOverflowCounter::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(AnalogMuxControlV2::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(IoDriverStrenghtConfiguration::ADDR, 0x0001_2111)
            .unwrap();
        bm1370.registers.insert(TimeOut::ADDR, 0x0000_FFFF).unwrap();
        bm1370
            .registers
            .insert(PLL1Parameter::ADDR, 0x2050_0174)
            .unwrap();
        bm1370
            .registers
            .insert(PLL2Parameter::ADDR, 0x2050_0174) // NOTE: Added by dwarloch
            .unwrap();
        bm1370
            .registers
            .insert(PLL3Parameter::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(OrderedClockMonitor::ADDR, 0x0001_0200)
            .unwrap();
        bm1370
            .registers
            .insert(PLL0Divider::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(PLL1Divider::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(PLL2Divider::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(PLL3Divider::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(ClockOrderControl0::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(ClockOrderControl1::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(ClockOrderStatus::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(FrequencySweepControl1::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(GoldenNonceForSweepReturn::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(ReturnedGroupPatternStatus::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(NonceReturnedTimeout::ADDR, 0x00f7_0073)
            .unwrap();
        bm1370
            .registers
            .insert(ReturnedSinglePatternStatus::ADDR, 0x0000_0000)
            .unwrap();
        bm1370
            .registers
            .insert(VersionRolling::ADDR, 0x0000_ffff)
            .unwrap();
        bm1370
            .registers
            .insert(HashCountingNumber::ADDR, 0x0000_1eb5)
            .unwrap();

        bm1370.registers.insert(RegA8::ADDR, 0x0007_0000).unwrap();
        bm1370.registers.insert(RegAC::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegB0::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegB4::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegB8::ADDR, 0x2000_0000).unwrap();
        bm1370.registers.insert(RegBC::ADDR, 0x0000_3313).unwrap();
        bm1370.registers.insert(RegC0::ADDR, 0x0000_2000).unwrap();
        bm1370.registers.insert(RegC4::ADDR, 0x0000_b850).unwrap();
        bm1370.registers.insert(RegC8::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegCC::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegD0::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegD4::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegD8::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegDC::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegE0::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegE4::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegE8::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegEC::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegF0::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegF4::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegF8::ADDR, 0x0000_0000).unwrap();
        bm1370.registers.insert(RegFC::ADDR, 0x0000_0000).unwrap();
        // Default Core Registers Value
        bm1370
            .core_registers
            .insert(ClockDelayCtrlV2::ID, 0x98)
            .unwrap();
        // bm1370.core_registers.insert(1, 0x00).unwrap(); // not used anywhere in official FW
        bm1370.core_registers.insert(2, 0x55).unwrap();
        bm1370.core_registers.insert(3, 0x00).unwrap();
        bm1370.core_registers.insert(4, 0x00).unwrap();
        bm1370
            .core_registers
            .insert(HashClockCtrl::ID, 0x40)
            .unwrap();
        bm1370
            .core_registers
            .insert(HashClockCounter::ID, 0x08)
            .unwrap();
        bm1370.core_registers.insert(7, 0x11).unwrap();
        bm1370.core_registers.insert(CoreReg8::ID, 0x00).unwrap();
        bm1370.core_registers.insert(CoreReg11::ID, 0x00).unwrap(); // TODO: Check initial value
        bm1370.core_registers.insert(15, 0x00).unwrap();
        bm1370.core_registers.insert(16, 0x00).unwrap();
        bm1370.core_registers.insert(CoreReg22::ID, 0x00).unwrap();
        bm1370
    }
}

impl Asic for BM1370 {
    /// ## Get the Chip ID
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use bm13xx_asic::Asic;
    ///
    /// let bm1370 = BM1370::default();
    /// assert_eq!(bm1370.chip_id(), 0x1370);
    /// ```
    fn chip_id(&self) -> u16 {
        BM1370_CHIP_ID
    }

    /// ## Has Version Rolling in chip
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use bm13xx_asic::Asic;
    ///
    /// let bm1370 = BM1370::default();
    /// assert!(bm1370.has_version_rolling());
    /// ```
    fn has_version_rolling(&self) -> bool {
        true
    }

    /// ## Init the Chip command list
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use bm13xx_asic::{core_register::*, register::*, Asic};
    ///
    /// let mut bm1370 = BM1370::default();
    /// let mut init_seq = bm1370.send_init(256, 1, 10, 2);
    /// assert_eq!(init_seq.len(), 8);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x41, 0x09, 0x12, 0x2c, 0x00, 0x18, 0x00, 0x03, 0x0c]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x41, 0x09, 0x00, 0x2c, 0x00, 0x18, 0x00, 0x03, 0x10]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x41, 0x09, 0x12, 0x58, 0x02, 0x11, 0xf1, 0x11, 0x1b]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x58, 0x02, 0x11, 0x11, 0x11, 0x06]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x54, 0x00, 0x00, 0x00, 0x03, 0x1d]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x14, 0x00, 0x00, 0x00, 0xff, 0x08]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x80, 0x0C, 0x11]);
    /// assert_eq!(init_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x8B, 0x00, 0x12]);
    /// assert_eq!(bm1370.core_registers.get(&HashClockCtrl::ID).unwrap(), &0x40);
    /// assert_eq!(bm1370.core_registers.get(&ClockDelayCtrlV2::ID).unwrap(), &0x20);
    /// assert_eq!(bm1370.registers.get(&TicketMask::ADDR).unwrap(), &0x0000_00ff);
    /// assert_eq!(bm1370.registers.get(&AnalogMuxControlV2::ADDR).unwrap(), &0x0000_0003);
    /// assert_eq!(bm1370.registers.get(&IoDriverStrenghtConfiguration::ADDR).unwrap(), &0x0211_1111);
    /// ```
    ///
    fn send_init(
        &mut self,
        initial_diffculty: u32,
        chain_domain_cnt: u8,
        domain_asic_cnt: u8,
        asic_addr_interval: u16,
    ) -> Vec<CmdDelay, 2048> {
        let mut init_seq = Vec::new();

        // Note: https://github.com/GPTechinno/bm13xx-rs/pull/3/files#r1856682733
        // 1 - [55, AA, 51, 09, 00, 3C, 80, 00, 8B, 00, 12]
        let reg11 = CoreReg11(*self.core_registers.get(&CoreReg11::ID).unwrap()).val();

        init_seq
            .push(CmdDelay {
                cmd: Command::write_reg(
                    CoreRegisterControl::ADDR,
                    CoreRegisterControl::write_core_reg(0, CoreReg11(reg11)),
                    Destination::All,
                ),
                delay_ms: 10,
            })
            .unwrap();

        // 2 - [55, AA, 51, 09, 00, 3C, 80, 00, 80, 0C, 11] // S21 Pro
        // 2 - [55, AA, 51, 09, 00, 3C, 80, 00, 80, 10, 12] // S21 XP // TODO: Check/ handle this
        let clk_dly_ctrl =
            ClockDelayCtrlV2(*self.core_registers.get(&ClockDelayCtrlV2::ID).unwrap())
                .set_ccdly(0)
                .set_pwth(2)
                .disable_sweep_frequency_mode()
                .val();
        init_seq
            .push(CmdDelay {
                cmd: Command::write_reg(
                    CoreRegisterControl::ADDR,
                    CoreRegisterControl::write_core_reg(0, ClockDelayCtrlV2(clk_dly_ctrl)),
                    Destination::All,
                ),
                delay_ms: 10,
            })
            .unwrap();
        self.core_registers
            .insert(ClockDelayCtrlV2::ID, clk_dly_ctrl)
            .unwrap();

        // 3 - [55, AA, 51, 09, 00, 14, 00, 00, 00, FF, 08]
        let tck_mask = TicketMask::from_difficulty(initial_diffculty).val();
        init_seq
            .push(CmdDelay {
                cmd: Command::write_reg(TicketMask::ADDR, tck_mask, Destination::All),
                delay_ms: 10,
            })
            .unwrap();
        self.registers.insert(TicketMask::ADDR, tck_mask).unwrap();

        init_seq
    }

    /// ## Send Baudrate command list
    ///
    /// ### Example
    /// ```
    /// use bm1370::{BM1370, BM1370_PLL_ID_UART};
    /// use bm13xx_asic::{register::*, Asic};
    ///
    /// let mut bm1370 = BM1370::default();
    ///
    /// let mut baud_seq = bm1370.send_baudrate(3_125_000);
    /// assert_eq!(baud_seq.len(), 2);
    /// assert_eq!(baud_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x28, 0x01, 0x30, 0x00, 0x00, 0x1a]); // FastUartConfiguration
    /// // TODO: Need to be added in future
    /// // here there are a bunch of UartRelay writing according to the Chain Voltage Domain stackup (let's forget them for now)
    ///
    /// assert_eq!(baud_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x68, 0x5a, 0xa5, 0x5a, 0xa5, 0x1c]); // Pll3Paramter
    /// // TODO: Need to be added in future
    /// // here there are a bunch of IoDriverStrenghtConfiguration writing according to the Chain Voltage Domain stackup (let's forget them for now)
    ///
    /// assert!(bm1370.plls[BM1370_PLL_ID_UART].enabled());
    /// assert_eq!(bm1370.registers.get(&PLL3Parameter::ADDR).unwrap(), &0x5aa5_5aa5);
    /// assert_eq!(bm1370.registers.get(&FastUARTConfigurationV2::ADDR).unwrap(), &0x0130_0000); // TODO: is it v2 or not ?
    ///
    /// ```
    // NOTE: This Example is correct for 1370
    // TODO: Rewrite it for 1370: https://github.com/GPTechinno/bm13xx-rs/pull/3#discussion_r1856655278
    fn send_baudrate(
        &mut self,
        baudrate: u32,
        chain_domain_cnt: u8,
        domain_asic_cnt: u8,
        asic_addr_interval: u16,
    ) -> Vec<CmdDelay, 800> {
        let mut baud_seq = Vec::new();

        // 8 - [55, AA, 51, 09, 00, 58, 02, 11, 11, 11, 06]
        let io_drv_st_cfg = 0x0001_1111; // TODO: split into IoDriverStrenghtConfiguration
        baud_seq
            .push(CmdDelay {
                cmd: Command::write_reg(
                    IoDriverStrenghtConfiguration::ADDR,
                    io_drv_st_cfg,
                    Destination::All,
                ),
                delay_ms: 0,
            })
            .unwrap();
        self.registers
            .insert(IoDriverStrenghtConfiguration::ADDR, io_drv_st_cfg)
            .unwrap();

        // last chip of each voltage domain should have IoDriverStrenghtConfiguration set to 0x0001_3111
        // (iterating voltage domain in decreasing chip address order)
        for dom in (0..chain_domain_cnt).rev() {
            baud_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        IoDriverStrenghtConfiguration::ADDR,
                        0x0001_3111,
                        // TODO: This one works fine but Fix this for other chip.
                        Destination::Chip(
                            (dom * domain_asic_cnt + domain_asic_cnt - 1)
                                * asic_addr_interval as u8,
                        ),
                    ),
                    delay_ms: 0,
                })
                .unwrap();
        }

        let pll3_parameters = 0x5aa5_5aa5;
        baud_seq
            .push(CmdDelay {
                cmd: Command::write_reg(PLL3Parameter::ADDR, pll3_parameters, Destination::All),
                delay_ms: 0,
            })
            .unwrap();
        self.registers
            .insert(PLL3Parameter::ADDR, pll3_parameters)
            .unwrap();

        // first and last chip of each voltage domain should have UARTRelay with GAP_CNT=domain_asic_num*(chain_domain_num-domain_i)+14 and RO_REL_EN=CO_REL_EN=1
        // (iterating voltage domain in decreasing chip address order)
        for dom in (0..chain_domain_cnt).rev() {
            // TODO: value match S21 XP but do not match S21 Pro - find out why
            let uart_relay = UARTRelay(*self.registers.get(&UARTRelay::ADDR).unwrap())
                .set_gap_cnt(
                    (domain_asic_cnt as u16) * ((chain_domain_cnt as u16) - (dom as u16)) + 14,
                )
                .enable_ro_relay()
                .enable_co_relay()
                .val();
            // TODO: do we need this if??? I do not think so
            baud_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        UARTRelay::ADDR,
                        uart_relay,
                        Destination::Chip((dom * domain_asic_cnt) * asic_addr_interval as u8),
                    ),
                    delay_ms: 0,
                })
                .unwrap();
            baud_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        UARTRelay::ADDR,
                        uart_relay,
                        Destination::Chip(
                            (dom * domain_asic_cnt + domain_asic_cnt - 1)
                                * asic_addr_interval as u8,
                        ),
                    ),
                    delay_ms: 0,
                })
                .unwrap();
        }
        // let fast_uart_cfg = 0x0130_0000;
        // baud_seq
        //     .push(CmdDelay {
        //         cmd: Command::write_reg(
        //             FastUARTConfigurationV2::ADDR,
        //             fast_uart_cfg,
        //             Destination::All,
        //         ),
        //         delay_ms: 0,
        //     })
        //     .unwrap();

        if baudrate <= self.input_clock_freq.raw() as u32 / 8 {
            let fbase = self.input_clock_freq.raw() as u32;
            let bt8d = (fbase / (8 * baudrate)) - 1;
            let fast_uart_cfg = FastUARTConfigurationV2(
                *self.registers.get(&FastUARTConfigurationV2::ADDR).unwrap(),
            )
            .set_bclk_sel(BaudrateClockSelectV2::Clki)
            .set_bt8d(bt8d as u8)
            .val();
            baud_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        FastUARTConfigurationV2::ADDR,
                        fast_uart_cfg,
                        Destination::All,
                    ),
                    delay_ms: 0,
                })
                .unwrap();
            self.registers
                .insert(FastUARTConfigurationV2::ADDR, fast_uart_cfg)
                .unwrap();
            // let pll3_param = self.plls[BM1370_PLL_ID_UART].disable().unlock().parameter();
            // baud_seq
            //     .push(CmdDelay {
            //         cmd: Command::write_reg(PLL3Parameter::ADDR, pll3_param, Destination::All),
            //         delay_ms: 0,
            //     })
            //     .unwrap();
            // self.registers
            //     .insert(PLL3Parameter::ADDR, pll3_param)
            //     .unwrap();
        } else {
            let pll3_div4 = 6;
            self.plls[BM1370_PLL_ID_UART]
                .lock()
                .enable()
                .set_fb_div(112)
                .set_ref_div(1)
                .set_post1_div(1)
                .set_post2_div(1)
                .set_out_div(BM1370_PLL_OUT_UART, pll3_div4);
            // self.plls[BM1370_PLL_ID_UART]
            //     .set_parameter(0xC070_0111)
            //     .set_out_div(BM1370_PLL_OUT_UART, pll3_div4);
            let fbase = self.plls[BM1370_PLL_ID_UART]
                .frequency(self.input_clock_freq, BM1370_PLL_OUT_UART)
                .raw();
            let pll3_param = self.plls[BM1370_PLL_ID_UART].parameter();
            baud_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(PLL3Parameter::ADDR, pll3_param, Destination::All),
                    delay_ms: 0,
                })
                .unwrap();
            self.registers
                .insert(PLL3Parameter::ADDR, pll3_param)
                .unwrap();
            // let bt8d = (fbase as u32 / (2 * baudrate)) - 1;
            // let fast_uart_cfg = FastUARTConfigurationV2(
            //     *self.registers.get(&FastUARTConfigurationV2::ADDR).unwrap(),
            // )
            // .set_pll3_div4(pll3_div4) // TODO: PLL3?
            // .set_bclk_sel(BaudrateClockSelectV2::Pll3)
            // .set_bt8d(bt8d as u8)
            // .val();
            // baud_seq
            //     .push(CmdDelay {
            //         cmd: Command::write_reg(
            //             FastUARTConfigurationV2::ADDR,
            //             fast_uart_cfg,
            //             Destination::All,
            //         ),
            //         delay_ms: 0,
            //     })
            //     .unwrap();
            // self.registers
            //     .insert(FastUARTConfigurationV2::ADDR, fast_uart_cfg)
            //     .unwrap();
        }
        baud_seq
    }

    /// ## Reset the Chip Cores command list
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use bm13xx_asic::{core_register::*, register::*, Asic};
    /// use bm13xx_protocol::command::Destination;
    ///
    /// let mut bm1370 = BM1370::default();
    ///
    /// let mut reset_seq = bm1370.send_reset_core(Destination::Chip(0));
    ///
    /// assert_eq!(reset_seq.len(), 5);
    /// assert_eq!(
    ///     reset_seq.pop().unwrap().cmd,
    ///     [0x55, 0xaa, 0x41, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x82, 0xaa, 0x05]
    /// );
    /// assert_eq!(
    ///     reset_seq.pop().unwrap().cmd,
    ///     [0x55, 0xaa, 0x41, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x80, 0x0C, 0x19]
    /// );
    /// assert_eq!(
    ///     reset_seq.pop().unwrap().cmd,
    ///     [0x55, 0xaa, 0x41, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x8B, 0x00, 0x1A]
    /// );
    /// assert_eq!(
    ///     reset_seq.pop().unwrap().cmd,
    ///     [0x55, 0xaa, 0x41, 0x09, 0x00, 0x18, 0xf0, 0x00, 0xc1, 0x00, 0x0c]
    /// );
    /// assert_eq!(
    ///     reset_seq.pop().unwrap().cmd,
    ///     [0x55, 0xaa, 0x41, 0x09, 0x00, 0xa8, 0x00, 0x07, 0x01, 0xf0, 0x15]
    /// );
    /// ```
    // Note: This example is correct for 1370
    // NOTE: Has been already rewrited for BM1370: https://github.com/GPTechinno/bm13xx-rs/pull/3#discussion_r1856678492
    fn send_reset_core(&mut self, dest: Destination) -> Vec<CmdDelay, 800> {
        let mut reset_seq = Vec::new();
        if dest == Destination::All {
            unimplemented!();
        } else {
            let reg_a8 = RegA8(*self.registers.get(&RegA8::ADDR).unwrap())
                .set_b8()
                .set_b7_4(0xf)
                .val();
            reset_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(RegA8::ADDR, reg_a8, dest),
                    delay_ms: 10,
                })
                .unwrap();
            self.registers.insert(RegA8::ADDR, reg_a8).unwrap();
            let misc = MiscControlV2(*self.registers.get(&MiscControlV2::ADDR).unwrap())
                .set_core_return_nonce(0xf)
                .set_b27_26(0)
                .set_b25_24(0)
                .set_b19_16(0)
                .val();
            reset_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(MiscControlV2::ADDR, misc, dest),
                    delay_ms: 10,
                })
                .unwrap();
            self.registers.insert(MiscControlV2::ADDR, misc).unwrap();

            // TODO: S21XP is same except ClockDelayCtrl = 0x10 (instead of 0x0C)... why ?
            let c_reg11 = CoreReg11(*self.core_registers.get(&CoreReg11::ID).unwrap()).val();
            reset_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        CoreRegisterControl::ADDR,
                        CoreRegisterControl::write_core_reg(0, CoreReg11(c_reg11)),
                        dest,
                    ),
                    delay_ms: 10,
                })
                .unwrap();
            self.core_registers.insert(CoreReg11::ID, c_reg11).unwrap();
            let clk_dly_ctrl = 0xC; // TODO: Rewrite as propper registers
            reset_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        CoreRegisterControl::ADDR,
                        CoreRegisterControl::write_core_reg(0, ClockDelayCtrlV2(clk_dly_ctrl)),
                        dest,
                    ),
                    delay_ms: 10,
                })
                .unwrap();
            self.core_registers
                .insert(ClockDelayCtrlV2::ID, clk_dly_ctrl)
                .unwrap();

            let core_reg2 = 0xAA; // TODO: Implement Register
            reset_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(
                        CoreRegisterControl::ADDR,
                        CoreRegisterControl::write_core_reg(0, CoreReg2(core_reg2)),
                        dest,
                    ),
                    delay_ms: 10,
                })
                .unwrap();
            self.core_registers.insert(CoreReg2::ID, core_reg2).unwrap();
        }
        reset_seq
    }

    fn between_reset_and_set_freq(&mut self) -> Vec<CmdDelay, 40> {
        let mut seq = Vec::new();
        seq.push(CmdDelay {
            cmd: Command::write_reg(0xB9, 0x0000_4480, Destination::All),
            delay_ms: 20,
        })
        .unwrap();
        seq.push(CmdDelay {
            cmd: Command::write_reg(AnalogMuxControlV2::ADDR, 0x0000_0002, Destination::All),
            delay_ms: 100,
        })
        .unwrap();
        seq.push(CmdDelay {
            cmd: Command::write_reg(0xB9, 0x0000_4480, Destination::All),
            delay_ms: 20,
        })
        .unwrap();
        seq.push(CmdDelay {
            cmd: Command::write_reg(CoreRegisterControl::ADDR, 0x8000_8DEE, Destination::All),
            delay_ms: 100,
        })
        .unwrap();
        seq
    }

    /// ## Send Hash Frequency command list
    ///
    /// ### Example
    /// ```
    /// use bm1370::{BM1370, BM1370_PLL_ID_HASH};
    /// use bm13xx_asic::{register::*, Asic};
    /// use fugit::HertzU64;
    ///
    /// let mut bm1370 = BM1370::default();
    /// let mut hash_freq_seq = bm1370.send_hash_freq(HertzU64::MHz(75));
    /// assert_eq!(hash_freq_seq.len(), 4);
    /// assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xa8, 0x02, 0x63, 0x14]);
    // assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xa5, 0x02, 0x54, 0x09]); // seen on S19XP, but equivalent
    /// assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xb0, 0x02, 0x73, 9]);
    /// assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xaf, 0x02, 0x64, 0x0d]);
    // assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xa2, 0x02, 0x55, 0x30]); // seen on S19XP, but equivalent
    /// assert_eq!(hash_freq_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x08, 0xc0, 0xb4, 0x02, 0x74, 29]);
    /// assert_eq!(bm1370.plls[BM1370_PLL_ID_HASH].parameter(), 0xc0a8_0263);
    /// ```
    fn send_hash_freq(&mut self, target_freq: HertzU64) -> Vec<CmdDelay, 800> {
        let mut hash_freq_seq = Vec::new();
        // if self.plls[BM1370_PLL_ID_HASH].out_div(BM1370_PLL_OUT_HASH) != 0 {
        //     self.plls[BM1370_PLL_ID_HASH].set_out_div(BM1370_PLL_OUT_HASH, 0);
        //     hash_freq_seq
        //         .push(CmdDelay {
        //             cmd: Command::write_reg(
        //                 PLL0Divider::ADDR,
        //                 self.plls[BM1370_PLL_ID_HASH].divider(),
        //                 Destination::All,
        //             ),
        //             delay_ms: 2,
        //         })
        //         .unwrap();
        //     self.registers
        //         .insert(PLL0Divider::ADDR, self.plls[BM1370_PLL_ID_HASH].divider())
        //         .unwrap();
        // }
        let mut freq = self.hash_freq();
        let mut long_delay = false;
        loop {
            freq += HertzU64::kHz(6250);
            if freq > target_freq {
                freq = target_freq;
            }
            self.set_hash_freq(freq);
            if freq > HertzU64::MHz(380) {
                long_delay = !long_delay;
            }
            let next_freq = bm1370_send_hash_frequency(freq.to_Hz() as f32 / 1_000_000.0, 0.001);
            hash_freq_seq
                .push(CmdDelay {
                    cmd: Command::write_reg(PLL0Parameter::ADDR, next_freq, Destination::All),
                    delay_ms: if long_delay { 2300 } else { 400 },
                })
                .unwrap();
            self.registers
                .insert(PLL0Parameter::ADDR, next_freq)
                .unwrap();
            if freq == target_freq {
                break;
            }
        }
        hash_freq_seq
    }

    /// ## Send Enable Version Rolling command list
    ///
    /// ### Example
    /// ```
    /// use bm1370::BM1370;
    /// use bm13xx_asic::Asic;
    ///
    /// let mut bm1370 = BM1370::default();
    /// let mut vers_roll_seq = bm1370.send_version_rolling(0x1fff_e000);
    /// assert_eq!(vers_roll_seq.len(), 2);
    /// assert_eq!(vers_roll_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0xa4, 0x90, 0x00, 0xff, 0xff, 0x1c]);
    /// assert_eq!(vers_roll_seq.pop().unwrap().cmd, [0x55, 0xaa, 0x51, 0x09, 0x00, 0x10, 0x00, 0x00, 0x15, 0x1c, 0x02]);
    /// ```
    fn send_version_rolling(
        &mut self,
        mask: u32,
        chain_domain_cnt: u8,
        domain_asic_cnt: u8,
        asic_addr_interval: u16,
    ) -> Vec<CmdDelay, 800> {
        let mut vers_roll_seq = Vec::new();

        // Set nounce offset
        for i in 0..chain_domain_cnt {
            for j in 0..domain_asic_cnt {
                let offset = (i * domain_asic_cnt + j) * asic_addr_interval as u8;
                let nonce_offset = 0x8000_0000
                    + (65_536 / (chain_domain_cnt * domain_asic_cnt) as u32)
                        * (i * domain_asic_cnt + j) as u32;
                vers_roll_seq
                    .push(CmdDelay {
                        cmd: Command::write_reg(
                            ChipNonceOffsetV2::ADDR,
                            nonce_offset,
                            Destination::Chip(offset),
                        ),
                        delay_ms: 0,
                    })
                    .unwrap();
            }
        }

        let hcn = 0x00001EB5;
        vers_roll_seq
            .push(CmdDelay {
                cmd: Command::write_reg(HashCountingNumber::ADDR, hcn, Destination::All),
                delay_ms: 1,
            })
            .unwrap();
        self.registers
            .insert(HashCountingNumber::ADDR, hcn)
            .unwrap();
        let vers_roll = VersionRolling(*self.registers.get(&VersionRolling::ADDR).unwrap())
            .enable()
            .set_mask(mask)
            .val();
        vers_roll_seq
            .push(CmdDelay {
                cmd: Command::write_reg(VersionRolling::ADDR, vers_roll, Destination::All),
                delay_ms: 1,
            })
            .unwrap();
        self.registers
            .insert(VersionRolling::ADDR, vers_roll)
            .unwrap();
        self.enable_version_rolling(mask);
        vers_roll_seq
    }
}

/// Copied out from ESP-Miner
fn bm1370_send_hash_frequency(target_freq: f32, max_diff: f32) -> u32 {
    let mut freqbuf: [u8; 4] = [0x40, 0xA0, 0x02, 0x41];
    let mut postdiv_min = 255;
    let mut postdiv2_min = 255;
    let mut best_freq = 0.0;
    let mut best_refdiv = 0u8;
    let mut best_fbdiv = 0u8;
    let mut best_postdiv1 = 0u8;
    let mut best_postdiv2 = 0u8;

    for refdiv in (1..=2).rev() {
        for postdiv1 in (1..=7).rev() {
            for postdiv2 in (1..=7).rev() {
                let fb_divider = ((target_freq / 25.0
                    * (refdiv as f32 * postdiv1 as f32 * postdiv2 as f32))
                    .round()) as u16;
                let newf =
                    25.0 * fb_divider as f32 / (refdiv as f32 * postdiv1 as f32 * postdiv2 as f32);

                if fb_divider >= 0xa0
                    && fb_divider <= 0xef
                    && (target_freq - newf).abs() < max_diff
                    && postdiv1 >= postdiv2
                    && postdiv1 * postdiv2 < postdiv_min
                    && postdiv2 <= postdiv2_min
                {
                    postdiv2_min = postdiv2;
                    postdiv_min = postdiv1 * postdiv2;
                    best_freq = newf;
                    best_refdiv = refdiv;
                    best_fbdiv = fb_divider as u8;
                    best_postdiv1 = postdiv1;
                    best_postdiv2 = postdiv2;
                }
            }
        }
    }

    freqbuf[0] = if best_fbdiv as f32 * 25.0 / best_refdiv as f32 >= 2400.0 {
        0x50
    } else {
        0x40
    };
    freqbuf[1] = best_fbdiv;
    freqbuf[2] = best_refdiv;
    freqbuf[3] = (((best_postdiv1 - 1) & 0xf) << 4) | ((best_postdiv2 - 1) & 0xf);
    u32::from_be_bytes(freqbuf)
}

#[cfg(test)]
mod S21_XP {
    use super::*;
    use bm13xx_asic::Asic;

    #[test]
    fn send_init() {
        let mut bm1370 = BM1370::default();

        let mut init_seq = bm1370.send_init(256, 13, 7, 2); // S21 XP

        assert_eq!(init_seq.len(), 593);
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x8B, 0x00, 0x12]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x3c, 0x80, 0x00, 0x80, 0x10, 0x12]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x14, 0x00, 0x00, 0x00, 0xFF, 0x08]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x54, 0x00, 0x00, 0x00, 0x03, 0x1D]
        );
        // IO Driver strenght beginning
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x58, 0x00, 0x01, 0x11, 0x11, 0x0D]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0xB4, 0x58, 0x00, 0x01, 0x31, 0x11, 0x00]
        );
        for _ in 0..11 {
            init_seq.remove(0);
        }
        // IO Driver strenght last command below
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0x0C, 0x58, 0x00, 0x01, 0x31, 0x11, 0x0E]
        );
        // PLL3
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x68, 0x5A, 0xA5, 0x5A, 0xA5, 0x1C]
        );
        // uart relay beginning
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0xA8, 0x2C, 0x00, 0x15, 0x00, 0x03, 0x14]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0xB4, 0x2C, 0x00, 0x15, 0x00, 0x03, 0x1F]
        );
        for _ in 0..22 {
            init_seq.remove(0);
        }
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0x00, 0x2C, 0x00, 0x69, 0x00, 0x03, 0x0D]
        );
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x41, 0x09, 0x0C, 0x2C, 0x00, 0x69, 0x00, 0x03, 0x05]
        );
        // Fast uart
        assert_eq!(
            init_seq.remove(0).cmd,
            [0x55, 0xaa, 0x51, 0x09, 0x00, 0x28, 0x01, 0x30, 0x00, 0x00, 0x1A]
        );

        // All frames has been tested
        assert_eq!(init_seq.len(), 0);
    }
}