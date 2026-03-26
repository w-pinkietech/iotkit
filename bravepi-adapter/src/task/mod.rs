//! BravePI adapter async task。
//!
//! シリアルポートからフレームを読み、AdapterEvent に変換して channel に送信する。
//! blocking serial I/O は専用スレッドで実行し、async 側と bytes channel で接続する。

mod convert;
mod event_loop;
mod handle;
mod reader;

pub use convert::frame_to_event;
pub use event_loop::event_loop;
pub use handle::{start, AdapterHandle};
