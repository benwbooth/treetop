//! treetop's engine and shared logic, used by both the Qt app (`treetop`)
//! and the terminal UI (`treetop-tui`). No Qt here: it builds as a static
//! musl binary.

pub mod actions;
pub mod columns;
pub mod command;
pub mod containers;
pub mod cooling;
pub mod engine;
pub mod gpu;
pub mod format;
pub mod inspect;
pub mod keys;
pub mod layout;
pub mod net;
pub mod processes;
pub mod root;
pub mod settings;
pub mod system;

pub use engine::Engine;
