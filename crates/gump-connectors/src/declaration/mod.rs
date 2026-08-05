//! Declaration normalize / sign / accept (D04 / FORMATS.md §12).
//!
//! Concurrent mutations compare the current generation and create exactly one
//! next generation. Never stores Capsule bytes or runtime plaintext.

mod accept;
mod normalize;
mod sign;
mod types;

pub use accept::{AcceptResult, DeclarationError, DeclarationLedger};
pub use normalize::normalize_declaration;
pub use sign::{sign_declaration, verify_declaration_signature, DECLARATION_SIG_DOMAIN};
pub use types::{
    DeclarationDraft, NormalizedDeclaration, OverrideProvenance, DECLARATION_SCHEMA,
};
