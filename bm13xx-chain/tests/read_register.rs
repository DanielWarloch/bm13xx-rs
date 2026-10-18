mod common;

use bm1370::{BM1370, BM1370_CHIP_ID, BM1370_CORE_SMALL_CORE_CNT};
use bm13xx_asic::register::{NonceErrorCounter, TotalHashCounter};
use bm13xx_protocol::{command::Destination, response::ResponseType};
use common::{enumerate, nonce_frame, reg_frame};

const SC: usize = BM1370_CORE_SMALL_CORE_CNT;

#[tokio::test]
async fn send_read_register_only_writes_the_command() {
    let (mut chain, uart, delay) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    chain
        .send_read_register(TotalHashCounter::ADDR, Destination::Chip(0x80))
        .await
        .unwrap();
    // 55 AA 42 05 <chip> <reg> <crc5>
    let tx = uart.take_tx();
    assert_eq!(tx.len(), 7);
    assert_eq!(&tx[..6], &[0x55, 0xAA, 0x42, 0x05, 0x80, 0x8C]);
    assert!(delay.0.borrow().is_empty(), "must not wait");
    // nothing received yet: poll returns None instead of blocking
    assert_eq!(chain.poll_response().await.unwrap(), None);
}

#[tokio::test]
async fn reply_is_polled_in_order_with_nonces() {
    let (mut chain, uart, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    chain
        .send_read_register(NonceErrorCounter::ADDR, Destination::Chip(0x80))
        .await
        .unwrap();
    uart.push_rx(&nonce_frame(0x1234_5678, 0, 0x21, SC));
    uart.push_rx(&reg_frame(0x80, NonceErrorCounter::ADDR, 42, SC));
    uart.push_rx(&nonce_frame(0x9abc_def0, 0, 0x31, SC));

    let mut got = Vec::new();
    for _ in 0..10 {
        if let Some(r) = chain.poll_response().await.unwrap() {
            got.push(r);
        }
    }
    assert_eq!(got.len(), 3, "{got:?}");
    assert!(matches!(&got[0], ResponseType::Job(j) if j.nonce == 0x1234_5678));
    match &got[1] {
        ResponseType::Reg(r) => {
            assert_eq!(r.chip_addr, 0x80);
            assert_eq!(r.reg_addr, NonceErrorCounter::ADDR);
            assert_eq!(r.reg_value, 42);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(&got[2], ResponseType::Job(j) if j.nonce == 0x9abc_def0));
}

#[tokio::test]
async fn blocking_read_register_still_works() {
    let (mut chain, uart, _) = enumerate(BM1370::default(), BM1370_CHIP_ID, 2).await;
    uart.push_rx(&nonce_frame(0x1234_5678, 0, 0x21, SC));
    uart.push_rx(&reg_frame(0x80, TotalHashCounter::ADDR, 0xdead_beef, SC));
    let v = chain
        .read_register(TotalHashCounter::ADDR, Destination::Chip(0x80))
        .await
        .unwrap();
    assert_eq!(v, 0xdead_beef);
    // (the interleaved nonce was consumed by read_register)
    assert_eq!(chain.poll_response().await.unwrap(), None);
}
