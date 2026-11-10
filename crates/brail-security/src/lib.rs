//! brail-security: encrypted stream-key storage.
//!
//! Stream keys are secrets equivalent to a password — leaking one lets
//! anyone impersonate the user's channel. They're stored via the Windows
//! Credential Manager (`CredWriteW`/`CredReadW`), which encrypts secrets
//! at rest under the user's Windows login credentials (DPAPI under the
//! hood) — the same store Windows itself uses for saved Wi-Fi passwords
//! and RDP credentials, rather than a bespoke encryption scheme this
//! project would have to justify the safety of on its own.

pub mod redact;
pub mod vault;

pub use redact::{redact_known_patterns, SecretRegistry};
pub use vault::CredentialVault;
