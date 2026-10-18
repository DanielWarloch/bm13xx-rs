mod common;

use bm1370::{BM1370, BM1370_CHIP_ID};
use bm13xx_asic::{
    register::{PLL0Divider, PLL0Parameter},
    FreqRamp,
};
use common::{enumerate, write_cmds};
use fugit::HertzU64;

#[tokio::test]
async fn whole_chain_ramp_down_is_stepped() {
    let (mut chain, uart, delay) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    let ramp = FreqRamp::new(HertzU64::kHz(6250), 100);

    chain
        .set_hash_freq_ramped(HertzU64::MHz(100), ramp)
        .await
        .unwrap();
    uart.take_tx();
    delay.0.borrow_mut().clear();

    chain
        .set_hash_freq_ramped(HertzU64::MHz(50), ramp)
        .await
        .unwrap();
    let cmds = write_cmds(&uart.take_tx());
    // divider, then 100 -> 50 MHz in 8 steps of 6.25 MHz
    assert_eq!(cmds.len(), 1 + 8);
    assert_eq!(cmds[0][5], PLL0Divider::ADDR);
    assert!(cmds[1..]
        .iter()
        .all(|c| c[2] == 0x51 && c[5] == PLL0Parameter::ADDR));
    let delays = delay.0.borrow().clone();
    assert_eq!(delays.iter().filter(|&&d| d == 100).count(), 8 + 1); // steps + final settle
}

#[tokio::test]
async fn chip_ramp_is_stepped_and_addressed() {
    let (mut chain, uart, delay) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    let chip = chain.asic_addr_interval as u8; // second chip
    let ramp = FreqRamp::new(HertzU64::MHz(25), 7);

    chain
        .set_hash_freq_chip_ramped(chip, HertzU64::MHz(525), HertzU64::MHz(400), ramp)
        .await
        .unwrap();
    let cmds = write_cmds(&uart.take_tx());
    // divider once, then 500, 475, 450, 425, 400
    assert_eq!(cmds.len(), 1 + 5);
    assert_eq!(cmds[0][5], PLL0Divider::ADDR);
    for c in &cmds {
        assert_eq!(c[2], 0x41, "single-chip write");
        assert_eq!(c[4], chip);
    }
    assert!(cmds[1..].iter().all(|c| c[5] == PLL0Parameter::ADDR));
    // last step is the same write as a direct per-chip set to the target
    let direct = {
        let (mut chain2, uart2, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
        chain2
            .set_hash_freq_chip(chip, HertzU64::MHz(400))
            .await
            .unwrap();
        write_cmds(&uart2.take_tx())
    };
    assert_eq!(cmds.last(), direct.last());
    assert_eq!(delay.0.borrow().iter().filter(|&&d| d == 7).count(), 5);
}

#[tokio::test]
async fn chip_ramp_up_and_zero_step_jump() {
    let (mut chain, uart, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 1).await;
    chain
        .set_hash_freq_chip_ramped(
            0,
            HertzU64::MHz(400),
            HertzU64::MHz(450),
            FreqRamp::new(HertzU64::MHz(25), 0),
        )
        .await
        .unwrap();
    assert_eq!(write_cmds(&uart.take_tx()).len(), 1 + 2);
    chain
        .set_hash_freq_chip_ramped(
            0,
            HertzU64::MHz(450),
            HertzU64::MHz(100),
            FreqRamp::new(HertzU64::Hz(0), 0),
        )
        .await
        .unwrap();
    assert_eq!(write_cmds(&uart.take_tx()).len(), 1 + 1);
}
