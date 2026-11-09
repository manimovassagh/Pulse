//! # pulse-compiler
//!
//! Compiles Pulse applications (TypeScript + JSX) to WebAssembly.
//!
//! Pipeline, one module per stage:
//!
//! ```text
//! source ─ lexer/parser ─► ast ─ lower (type check) ─► ir ─ optimize ─► ir ─ codegen ─► Rust ─ driver (cargo) ─► .wasm
//! ```
//!
//! [`driver`] ties the stages together; `src/main.rs` is the `pulse` CLI.

pub mod ast;
pub mod codegen;
pub mod diagnostic;
pub mod driver;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod optimize;
pub mod parser;
