//! BR Foto: the simple, CapCut-style photo editing layer of BR, on LightCraft's engine.
//!
//! Everything here works on a [`lightcraft_engine::Session`] and is shared by the BR window (Simple
//! mode), the `br` command line and `br mcp`:
//!
//! - [`looks`]: one-tap Looks (LightCraft's built-in presets) with an Amount, as one undo step.
//! - [`adjust`]: six simple sliders (Licht, Kontrast, Farbe, Wärme, Schärfe, Vignette), each a
//!   fixed mix of LightCraft develop controls.
//! - [`crop`]: formats (Original, 1:1, 4:5, 9:16, 16:9) cropped to whole pixels, so exports come
//!   out at exactly the size a preset promises.
//! - [`export`]: export presets (Instagram, Story, Quadrat, Breitbild, volle Auflösung).
//! - [`commands`]: the `br.*` commands that the CLI and the MCP server dispatch by id.
//! - [`library`]: where BR keeps its library and how a session is opened.
//!
//! Edits go through `Session::execute` / `Session::execute_fn`, so they are undoable, saved to the
//! open library before the call returns, and visible to LightCraft's Pro interface.

pub mod adjust;
pub mod commands;
pub mod crop;
pub mod export;
pub mod library;
pub mod looks;
pub mod photo;

pub use commands::BrState;
pub use lightcraft_engine::{EngineError, Session};
