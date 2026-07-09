//! liteclaw-fs: filesystem claws (read / grep / edit).
//!
//! These are the everyday file tools. Each implements the [`Claw`] trait from
//! `liteclaw-core` and inherits Defender pre-checks + sandboxing via the shared
//! [`Ctx`].

mod edit;
mod grep;
mod read;

pub use edit::EditClaw;
pub use grep::GrepClaw;
pub use read::ReadClaw;
