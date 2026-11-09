//! # pulse-compiler
//!
//! Compiles Pulse applications (TypeScript + JSX) to WebAssembly.
//!
//! Pipeline, one module per stage:
//!
//! ```text
//! source ─ lexer/parser ─► ast ─ lower (type check) ─► ir ─ optimize ─► ir ─ codegen ─► Rust
//! ```
//!
//! The crate is pure: it turns source text into Rust source text. Building
//! that into WebAssembly is `pulse-build`'s job.

pub mod ast;
pub mod codegen;
pub mod diagnostic;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod optimize;
pub mod parser;

pub use codegen::CodegenOptions;

/// Compiles TSX source to Rust source.
///
/// Errors are returned rendered for humans, with file, line, column and a
/// caret under the offending code.
pub fn compile(source: &str, opts: &CodegenOptions) -> Result<String, String> {
    let render = |d: diagnostic::Diagnostic| d.render(&opts.source_name, source);
    let program = parser::parse(source).map_err(render)?;
    let mut module = lower::lower(&program).map_err(render)?;
    optimize::optimize(&mut module);
    Ok(codegen::generate(&module, opts))
}

/// Parses and type-checks TSX source without generating code.
pub fn check(source: &str, file_name: &str) -> Result<(), String> {
    let render = |d: diagnostic::Diagnostic| d.render(file_name, source);
    lower::lower(&parser::parse(source).map_err(render)?).map_err(render)?;
    Ok(())
}
