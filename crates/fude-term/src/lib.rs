//! GUI に依存しないターミナルの中身。VT の解析と画面バッファ、PTY のセッションを扱う。

pub mod input;
pub mod palette;
pub mod session;
pub mod shell;
pub mod state;
pub mod wsl;

pub use input::{WheelAccum, alt_scroll_bytes, alt_scroll_lines, paste_bytes};
pub use palette::Palette;
pub use session::{Session, SessionEvent};
pub use shell::{SpawnSpec, spawn_spec};
pub use state::{CursorKind, GridSize, SelectKind, SnapCell, SnapCursor, Snapshot, TermEvent, TermState};
pub use wsl::{list_distros, parse_distro_list};
