//! # pulse-runtime
//!
//! The runtime linked into every Pulse application. It provides:
//!
//! * [`reactive`] — signals, computed values, effects and batching with
//!   automatic dependency tracking;
//! * [`dom`] — direct DOM manipulation through a minimal WebAssembly host
//!   interface (with an in-memory DOM for native tests);
//! * [`hot`] — state transfer for hot reloading in development;
//! * [`text`] — JavaScript-compatible value-to-text conversion.
//!
//! Application code does not use this crate directly: the Pulse compiler
//! translates TSX components into Rust that calls into it.

pub mod dom;
pub mod hot;
pub mod reactive;
pub mod text;

/// Everything generated code needs, in one import.
pub mod prelude {
    pub use crate::dom::{
        append_child, console_log, create_element, create_text, mount, on, set_attribute,
        set_bool_attribute, set_text, Node,
    };
    pub use crate::reactive::{batch, computed, effect, signal, untrack, Computed, Signal};
    pub use crate::text::ToText;
    pub use std::rc::Rc;
}
