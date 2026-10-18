mod common;

use bm1370::{BM1370, BM1370_CHIP_ID, BM1370_CORE_SMALL_CORE_CNT};
use bm13xx_asic::register::ChipIdentification;
use bm13xx_chain::Chain;
use bm13xx_protocol::response::ResponseType;
use common::{nonce_frame, reg_frame, MockDelay, MockPin, MockUart};

const SC: usize = BM1370_CORE_SMALL_CORE_CNT;

#[tokio::test]
async fn enumerate_counts_all_chips() {
    let uart = MockUart::default();
    for _ in 0..4 {
        uart.push_rx(&reg_frame(0, ChipIdentification::ADDR, 0x1370_0000, SC));
    }
    let chain = Chain::enumerate(
        BM1370::default(),
        uart.clone(),
        MockPin,
        MockPin,
        MockDelay::default(),
    )
    .await
    .unwrap();
    assert_eq!(chain.asic_cnt, 4);
    assert_eq!(chain.asic_addr_interval, 64);
    let _ = BM1370_CHIP_ID;
}

#[tokio::test]
async fn poll_response_returns_a_frame_in_the_call_that_reads_it() {
    let (mut chain, uart, _) = common::enumerate(BM1370::default(), BM1370_CHIP_ID, 1).await;
    uart.push_rx(&nonce_frame(0x1234_5678, 0, 0x21, SC));
    match chain.poll_response().await.unwrap() {
        Some(ResponseType::Job(j)) => assert_eq!(j.nonce, 0x1234_5678),
        other => panic!("{other:?}"),
    }
    assert_eq!(chain.poll_response().await.unwrap(), None);
}
