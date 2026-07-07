//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
pub(crate) mod event_loop;
mod handle;
pub(crate) mod ingest_map;
mod serial_source;

pub use handle::{AdapterHandle, AdapterParts, ShutdownHandle, start};

#[cfg(test)]
mod convert_test;
#[cfg(test)]
mod event_loop_test;
