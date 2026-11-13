//! GUI に依存しないターミナルの中身。VT の解析と画面バッファ、PTY のセッションを扱う。

pub mod input;
pub mod palette;
pub mod state;

pub use input::{WheelAccum, alt_scroll_bytes, paste_bytes};
pub use palette::Palette;
pub use state::{CursorKind, GridSize, SelectKind, SnapCell, SnapCursor, Snapshot, TermEvent, TermState};
