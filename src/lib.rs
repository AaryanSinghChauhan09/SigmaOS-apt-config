//! SigmaOS-apt-config — apt.conf-style scoped configuration store.
//!
//! Inspired by Debian's `apt-config`: parses the `apt.conf` configuration
//! language — scoped keys joined by `::`, `//` comments, quoted or bare
//! values, and `{ item; item; };` list blocks — into a flat map, and supports
//! `get`/`set`/`dump`. This is the mechanism half of apt configuration; policy
//! (which keys are valid) lives in callers, keeping mechanism and policy
//! separate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::collections::BTreeMap;
use std::fmt;

/// Errors raised by this component.
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    /// A configuration line could not be parsed.
    #[error("malformed config line: {0}")]
    Malformed(String),
    /// A list block was opened but never closed.
    #[error("unterminated list block")]
    UnterminatedList,
}

/// Result alias for component operations.
pub type ComponentResult<T> = Result<T, ComponentError>;

/// A parsed configuration value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// A scalar (quoted string content or bare token).
    Scalar(String),
    /// A `{ a; b; c; };` list of scalars.
    List(Vec<String>),
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Scalar(s) => write!(f, "\"{}\"", s),
            Value::List(items) => {
                write!(f, "{{")?;
                for it in items {
                    write!(f, " \"{}\";", it)?;
                }
                write!(f, " }}")
            }
        }
    }
}

/// Flat map of scoped config key → value. Keys use the `::` separator
/// (`Dir::Etc::SourceList`). `BTreeMap` keeps `dump` output deterministic.
#[derive(Debug, Default)]
pub struct Config {
    map: BTreeMap<String, Value>,
}

impl Config {
    /// Empty configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `apt.conf` text into a `Config`.
    pub fn parse(input: &str) -> ComponentResult<Self> {
        let mut cfg = Config::new();
        // Strip `//` comments line-wise (apt.conf has no block comments).
        for raw in input.lines() {
            let line = raw.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            cfg.parse_line(line)?;
        }
        Ok(cfg)
    }

    fn parse_line(&mut self, line: &str) -> ComponentResult<()> {
        // Split at the first `;` that is not inside quotes/braces is complex;
        // apt.conf statements end with `;`. Find the trailing `;`.
        let stmt = line.strip_suffix(';').unwrap_or(line).trim();
        if stmt.is_empty() {
            return Ok(());
        }
        // Key is everything up to the first whitespace.
        let (key, rest) = match stmt.split_once(char::is_whitespace) {
            Some((k, r)) => (k.trim(), r.trim()),
            None => return Err(ComponentError::Malformed(stmt.to_string())),
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '-' || c == '_') {
            return Err(ComponentError::Malformed(stmt.to_string()));
        }
        if rest.is_empty() {
            return Err(ComponentError::Malformed(stmt.to_string()));
        }
        let value = parse_value(rest)?;
        self.map.insert(key.to_string(), value);
        Ok(())
    }

    /// Get a value by scoped key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key)
    }

    /// Get a scalar value as `&str` (empty for lists).
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.map.get(key) {
            Some(Value::Scalar(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Set a scalar value.
    pub fn set(&mut self, key: &str, value: &str) {
        self.map.insert(key.to_string(), Value::Scalar(value.to_string()));
    }

    /// Set a list value.
    pub fn set_list(&mut self, key: &str, items: Vec<String>) {
        self.map.insert(key.to_string(), Value::List(items));
    }

    /// Remove a key.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.map.remove(key)
    }

    /// Number of keys.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All keys in sorted order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// Render back to apt.conf text (sorted by key for determinism).
    pub fn dump(&self) -> String {
        let mut out = String::new();
        for (k, v) in &self.map {
            out.push_str(k);
            out.push(' ');
            out.push_str(&v.to_string());
            out.push_str(";\n");
        }
        out
    }
}

fn parse_value(rest: &str) -> ComponentResult<Value> {
    let rest = rest.trim();
    if rest.starts_with('{') {
        // List block: { "a"; "b"; } or { a; b; }
        let inner = rest
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .ok_or_else(|| ComponentError::UnterminatedList)?;
        let mut items = Vec::new();
        for part in inner.split(';') {
            let p = part.trim().trim_matches('"').trim();
            if !p.is_empty() {
                items.push(p.to_string());
            }
        }
        return Ok(Value::List(items));
    }
    // Scalar: strip surrounding quotes if present.
    let scalar = if rest.starts_with('"') && rest.ends_with('"') && rest.len() >= 2 {
        rest[1..rest.len() - 1].to_string()
    } else {
        rest.to_string()
    };
    Ok(Value::Scalar(scalar))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "// comment\nDir::Etc::SourceList \"/etc/apt/sources.list\";\nDir::Etc::SourceParts \"/etc/apt/sources.list.d\";\nAPT::Keep-Downloaded-Packages \"true\";\nDebug::NoLocking 1;\n";

    #[test]
    fn parses_scoped_keys() {
        let c = Config::parse(SAMPLE).unwrap();
        assert_eq!(c.get_str("Dir::Etc::SourceList"), Some("/etc/apt/sources.list"));
        assert_eq!(c.get_str("APT::Keep-Downloaded-Packages"), Some("true"));
        assert_eq!(c.get_str("Debug::NoLocking"), Some("1"));
    }

    #[test]
    fn comments_and_blanks_ignored() {
        let c = Config::parse("// only a comment\n\n   \n").unwrap();
        assert!(c.is_empty());
    }

    #[test]
    fn set_and_get_roundtrip() {
        let mut c = Config::new();
        c.set("Foo::Bar", "baz");
        assert_eq!(c.get_str("Foo::Bar"), Some("baz"));
        c.remove("Foo::Bar");
        assert!(c.get("Foo::Bar").is_none());
    }

    #[test]
    fn list_value_parses() {
        let c = Config::parse("Dir::Bin::Methods { \"cdrom\"; \"http\"; \"https\"; };").unwrap();
        match c.get("Dir::Bin::Methods") {
            Some(Value::List(items)) => {
                let got: Vec<&str> = items.iter().map(String::as_str).collect();
                assert_eq!(got, vec!["cdrom", "http", "https"]);
            }
            other => panic!("expected list, got {:?}", other),
        }
    }

    #[test]
    fn dump_is_deterministic_sorted() {
        let c = Config::parse("B::Two 2;\nA::One 1;\n").unwrap();
        let d = c.dump();
        assert!(d.find("A::One").unwrap() < d.find("B::Two").unwrap());
    }

    #[test]
    fn malformed_line_errors() {
        assert!(Config::parse("justkey\n").is_err());
    }
}
