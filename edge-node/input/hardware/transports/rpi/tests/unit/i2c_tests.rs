use super::{I2cDevice, require_exact_transfer_count};
use std::io;

#[derive(Debug, PartialEq, Eq)]
enum Transaction {
    Read(usize),
    Write(Vec<u8>),
    WriteRead(Vec<u8>, usize),
}

#[derive(Default)]
struct RecordingDevice {
    transactions: Vec<Transaction>,
}

impl I2cDevice for RecordingDevice {
    fn read(&mut self, data: &mut [u8]) -> io::Result<()> {
        self.transactions.push(Transaction::Read(data.len()));
        data.fill(0);
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> io::Result<()> {
        self.transactions.push(Transaction::Write(data.to_vec()));
        Ok(())
    }

    fn write_read(&mut self, write: &[u8], read: &mut [u8]) -> io::Result<()> {
        self.transactions
            .push(Transaction::WriteRead(write.to_vec(), read.len()));
        read.copy_from_slice(&[0x12, 0x34]);
        Ok(())
    }
}

#[test]
fn register_read_uses_one_combined_transaction() {
    let mut device = RecordingDevice::default();
    let mut data = [0_u8; 2];

    device.read_register(0x0f, &mut data).unwrap();

    assert_eq!(data, [0x12, 0x34]);
    assert_eq!(device.transactions, [Transaction::WriteRead(vec![0x0f], 2)]);
}

#[test]
fn register_write_is_one_raw_write() {
    let mut device = RecordingDevice::default();

    device.write_register(0x05, &[0xaa, 0xbb]).unwrap();

    assert_eq!(
        device.transactions,
        [Transaction::Write(vec![0x05, 0xaa, 0xbb])]
    );
}

#[test]
fn partial_combined_transfer_is_an_error() {
    let error = require_exact_transfer_count(1, 2).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn partial_single_transfer_is_an_error() {
    let error = require_exact_transfer_count(0, 1).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}
