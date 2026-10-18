mod common;

use bm1370::{BM1370, BM1370_CHIP_ID};
use bm13xx_asic::register::TicketMask;
use common::{enumerate, write_cmds};

#[tokio::test]
async fn set_ticket_mask_broadcasts_the_mask() {
    let (mut chain, uart, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    chain.set_ticket_mask(512).await.unwrap();
    let cmds = write_cmds(&uart.take_tx());
    assert_eq!(cmds.len(), 1);
    // 55 AA 51 09 00 14 <mask BE>: broadcast write of TicketMask
    assert_eq!(&cmds[0][2..6], &[0x51, 0x09, 0x00, TicketMask::ADDR]);
    assert_eq!(
        u32::from_be_bytes(cmds[0][6..10].try_into().unwrap()),
        0x0000_80ff
    );
    // difficulty 0 is clamped to 1 (all nonces) instead of underflowing
    chain.set_ticket_mask(0).await.unwrap();
    let cmds = write_cmds(&uart.take_tx());
    assert_eq!(u32::from_be_bytes(cmds[0][6..10].try_into().unwrap()), 0);
}

#[tokio::test]
async fn set_hash_freq_chip_addresses_one_chip() {
    use bm13xx_asic::{
        register::{PLL0Divider, PLL0Parameter},
        Asic,
    };
    use bm13xx_protocol::command::Destination;
    use fugit::HertzU64;

    let (mut chain, uart, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    let chip = chain.asic_addr_interval as u8;
    chain
        .set_hash_freq_chip(chip, HertzU64::MHz(475))
        .await
        .unwrap();
    let cmds = write_cmds(&uart.take_tx());
    assert_eq!(cmds.len(), 2);
    for c in &cmds {
        assert_eq!(c[2], 0x41, "single-chip write");
        assert_eq!(c[4], chip);
    }
    assert_eq!(cmds[0][5], PLL0Divider::ADDR);
    assert_eq!(cmds[1][5], PLL0Parameter::ADDR);
    // same bytes as the Asic hook, which leaves the chain-nominal frequency alone
    let asic = BM1370::default();
    let (div, par) = asic
        .set_hash_freq_chip_cmd(Destination::Chip(chip), HertzU64::MHz(475))
        .unwrap();
    assert_eq!((cmds[0], cmds[1]), (div.cmd, par.cmd));
    assert_eq!(asic.hash_freq(), HertzU64::MHz(50));
}
