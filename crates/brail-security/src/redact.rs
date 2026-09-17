use std::collections::HashSet;
use std::sync::{Arc, RwLock};

/// Registry of secrets that must never appear in a log line (§32, §37).
///
/// Redaction is enforced at the *writer* layer rather than at each call
/// site, because relying on every future `tracing::info!` to remember not
/// to include a key is exactly the kind of discipline that fails once and
/// leaks a credential permanently into a log file a user then attaches to
/// a bug report. Registering a secret here means it cannot be written even
/// if some other code path formats it into a message by mistake.
#[derive(Clone, Default)]
pub struct SecretRegistry {
    secrets: Arc<RwLock<HashSet<String>>>,
}

impl SecretRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a secret for redaction. Very short values are ignored:
    /// redacting a 3-character string would mangle unrelated log text
    /// wherever those characters happened to appear, and a secret that
    /// short isn't protecting anything anyway.
    pub fn register(&self, secret: &str) {
        if secret.len() < 8 {
            return;
        }
        if let Ok(mut set) = self.secrets.write() {
            set.insert(secret.to_string());
        }
    }

    pub fn forget(&self, secret: &str) {
        if let Ok(mut set) = self.secrets.write() {
            set.remove(secret);
        }
    }

    /// Replaces every registered secret in `text` with a redaction marker.
    pub fn redact(&self, text: &str) -> String {
        let Ok(set) = self.secrets.read() else {
            // A poisoned lock means another thread panicked mid-write. The
            // safe response is to redact everything rather than risk
            // emitting an unredacted line.
            return "[redacted: secret registry unavailable]".to_string();
        };

        let mut result = text.to_string();
        for secret in set.iter() {
            if result.contains(secret.as_str()) {
                result = result.replace(secret.as_str(), "[REDACTED]");
            }
        }
        result
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.read().map(|s| s.is_empty()).unwrap_or(false)
    }
}

/// Patterns that look like credentials regardless of whether they were
/// explicitly registered — a defense-in-depth pass for keys that reach a
/// log before anything registered them.
pub fn redact_known_patterns(text: &str) -> String {
    let mut out = String::with_capacity(text.len());

    for token in text.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim();

        // YouTube stream keys are four dash-separated groups of
        // alphanumerics; Twitch keys start with a "live_" prefix.
        let looks_like_youtube_key = trimmed.len() >= 16
            && trimmed.matches('-').count() >= 3
            && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        let looks_like_twitch_key = trimmed.starts_with("live_") && trimmed.len() > 20;

        if looks_like_youtube_key || looks_like_twitch_key {
            out.push_str("[REDACTED]");
            if token.ends_with(char::is_whitespace) {
                out.push(' ');
            }
        } else {
            out.push_str(token);
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_secret_is_removed() {
        let registry = SecretRegistry::new();
        registry.register("abcd-efgh-ijkl-mnop");
        let out = registry.redact("connecting with key abcd-efgh-ijkl-mnop now");
        assert!(!out.contains("abcd-efgh-ijkl-mnop"));
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn short_values_are_not_registered() {
        let registry = SecretRegistry::new();
        registry.register("abc");
        // "abc" must not be redacted out of unrelated words like "abcdef".
        assert_eq!(registry.redact("abcdef"), "abcdef");
    }

    #[test]
    fn unregistered_key_shaped_tokens_are_caught() {
        let out = redact_known_patterns("key is a1b2-c3d4-e5f6-g7h8 ok");
        assert!(!out.contains("a1b2-c3d4-e5f6-g7h8"));
    }

    #[test]
    fn ordinary_text_is_untouched() {
        let out = redact_known_patterns("recording started at 1920x1080 60fps");
        assert_eq!(out, "recording started at 1920x1080 60fps");
    }
}
