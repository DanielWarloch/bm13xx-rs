//! Host-side mocks to drive a `Chain` without hardware.
#![allow(dead_code)]

use std::{cell::RefCell, collections::VecDeque, convert::Infallible, rc::Rc, vec::Vec};

use bm13xx_asic::Asic;
use bm13xx_chain::{Baud, Chain};
use bm13xx_protocol::response::{Response, FRAME_SIZE};

/// UART mock: bytes queued in `rx` are returned by `read`, writes land in `tx`.
#[derive(Debug, Default)]
pub struct Wire {
    pub rx: VecDeque<u8>,
    pub tx: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct MockUart(pub Rc<RefCell<Wire>>);

impl PartialEq for MockUart {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl MockUart {
    pub fn push_rx(&self, bytes: &[u8]) {
        self.0.borrow_mut().rx.extend(bytes.iter().copied());
    }
    pub fn take_tx(&self) -> Vec<u8> {
        core::mem::take(&mut self.0.borrow_mut().tx)
    }
}

impl embedded_io_async::ErrorType for MockUart {
    type Error = Infallible;
}

impl embedded_io_async::Read for MockUart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        let mut wire = self.0.borrow_mut();
        let n = buf.len().min(wire.rx.len());
        for b in buf.iter_mut().take(n) {
            *b = wire.rx.pop_front().unwrap();
        }
        Ok(n)
    }
}

impl embedded_io_async::ReadReady for MockUart {
    fn read_ready(&mut self) -> Result<bool, Self::Error> {
        Ok(!self.0.borrow().rx.is_empty())
    }
}

impl embedded_io_async::Write for MockUart {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.0.borrow_mut().tx.extend_from_slice(buf);
        Ok(buf.len())
    }
}

impl Baud for MockUart {
    fn set_baudrate(&mut self, _baudrate: u32) {}
}

#[derive(Debug, Default, PartialEq)]
pub struct MockPin;

impl embedded_hal::digital::ErrorType for MockPin {
    type Error = Infallible;
}

impl embedded_hal::digital::OutputPin for MockPin {
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Delay mock: records the requested delays (in ms, rounded down) and returns at once.
#[derive(Debug, Clone, Default)]
pub struct MockDelay(pub Rc<RefCell<Vec<u32>>>);

impl PartialEq for MockDelay {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl embedded_hal_async::delay::DelayNs for MockDelay {
    async fn delay_ns(&mut self, ns: u32) {
        self.0.borrow_mut().push(ns / 1_000_000);
    }
}

/// A 9-byte register response frame (non version-rolling), with a valid CRC5.
pub fn reg_frame(chip_addr: u8, reg_addr: u8, value: u32, core_small_core_cnt: usize) -> [u8; 9] {
    let v = value.to_be_bytes();
    let mut f = [0xAA, 0x55, v[0], v[1], v[2], v[3], chip_addr, reg_addr, 0];
    with_crc(&mut f, 0x00, core_small_core_cnt);
    f
}

/// A 9-byte nonce (job) response frame (non version-rolling), with a valid CRC5.
pub fn nonce_frame(
    nonce: u32,
    midstate_id: u8,
    job_byte: u8,
    core_small_core_cnt: usize,
) -> [u8; 9] {
    let n = nonce.to_le_bytes();
    let mut f = [0xAA, 0x55, n[0], n[1], n[2], n[3], midstate_id, job_byte, 0];
    with_crc(&mut f, 0x80, core_small_core_cnt);
    f
}

fn with_crc(f: &mut [u8; FRAME_SIZE], flags: u8, core_small_core_cnt: usize) {
    for crc in 0..32u8 {
        f[8] = flags | crc;
        if Response::parse(f, core_small_core_cnt).is_ok() {
            return;
        }
    }
    panic!("no valid CRC5 found");
}

pub type MockChain<A> = Chain<A, MockUart, MockPin, MockPin, MockDelay>;

/// Enumerate a mocked chain of `asic_cnt` chips of `chip_id`, then clear the wire.
pub async fn enumerate<A: Asic>(
    asic: A,
    chip_id: u16,
    asic_cnt: usize,
) -> (MockChain<A>, MockUart, MockDelay) {
    let uart = MockUart::default();
    let delay = MockDelay::default();
    let ident = (chip_id as u32) << 16;
    for _ in 0..asic_cnt {
        uart.push_rx(&reg_frame(0, 0x00, ident, asic.core_small_core_count()));
    }
    let chain = Chain::enumerate(asic, uart.clone(), MockPin, MockPin, delay.clone())
        .await
        .expect("mock enumeration");
    assert_eq!(chain.asic_cnt, asic_cnt);
    uart.take_tx();
    delay.0.borrow_mut().clear();
    (chain, uart, delay)
}

/// Split a write log into 11-byte register-write commands (`55 AA 41/51 ...`).
pub fn write_cmds(tx: &[u8]) -> Vec<[u8; 11]> {
    assert_eq!(tx.len() % 11, 0, "not only register writes: {tx:02x?}");
    tx.chunks_exact(11)
        .map(|c| {
            assert_eq!(&c[..2], &[0x55, 0xAA]);
            c.try_into().unwrap()
        })
        .collect()
}
