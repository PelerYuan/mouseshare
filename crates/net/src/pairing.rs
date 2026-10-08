//! The pairing code: the one shared secret a user ever has to type.
//!
//! Codes are 10 characters from Crockford's base32 alphabet (no `I L O U`,
//! so they survive being read aloud or copied from a phone), shown as
//! `XXXXX-XXXXX`. That is 50 bits -- plenty, because the code is used as a
//! SPAKE2 password: an attacker gets exactly one guess per connection
//! attempt and learns nothing offline (see `secure.rs`).

use std::fmt;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CODE_LEN: usize = 10;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PairingCodeError {
    #[error("a pairing code is {CODE_LEN} letters/digits (e.g. K7QDM-2XP9R)")]
    BadLength,
    #[error("pairing codes only contain letters and digits")]
    BadCharacter,
}

/// A normalized pairing code. Construct with [`PairingCode::generate`] or
/// [`PairingCode::parse`]; both guarantee the canonical form, so two codes
/// typed with different punctuation/case compare equal.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode(String);

impl PairingCode {
    /// A fresh random code.
    pub fn generate() -> Self {
        let mut bytes = [0u8; CODE_LEN];
        getrandom::getrandom(&mut bytes).expect("the OS random source is unavailable");
        let code: String = bytes
            .iter()
            .map(|b| ALPHABET[(*b & 31) as usize] as char)
            .collect();
        Self(code)
    }

    /// Parses user input: case-insensitive, ignores spaces and dashes, and
    /// maps the usual look-alikes (`O`→`0`, `I`/`L`→`1`) the way Crockford
    /// base32 specifies.
    pub fn parse(input: &str) -> Result<Self, PairingCodeError> {
        let mut out = String::with_capacity(CODE_LEN);
        for ch in input.chars() {
            if ch == '-' || ch.is_whitespace() {
                continue;
            }
            let up = ch.to_ascii_uppercase();
            let mapped = match up {
                'O' => '0',
                'I' | 'L' => '1',
                other => other,
            };
            if !ALPHABET.contains(&(mapped as u8)) || !mapped.is_ascii() {
                return Err(PairingCodeError::BadCharacter);
            }
            out.push(mapped);
        }
        if out.len() != CODE_LEN {
            return Err(PairingCodeError::BadLength);
        }
        Ok(Self(out))
    }

    /// The canonical, undecorated form (what is fed to the key exchange).
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// The form shown to humans: `XXXXX-XXXXX`.
    pub fn display(&self) -> String {
        format!("{}-{}", &self.0[..5], &self.0[5..])
    }
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// Deliberately does not print the secret.
impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_round_trip_through_parse() {
        for _ in 0..50 {
            let c = PairingCode::generate();
            assert_eq!(PairingCode::parse(&c.display()).unwrap(), c);
        }
    }

    #[test]
    fn parse_is_forgiving_about_case_spacing_and_lookalikes() {
        let a = PairingCode::parse("k7qdm-2xp9r").unwrap();
        let b = PairingCode::parse(" K7QDM 2XP9R ").unwrap();
        assert_eq!(a, b);
        assert_eq!(
            PairingCode::parse("OILOILOILO").unwrap(),
            PairingCode::parse("0110110110").unwrap()
        );
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert_eq!(PairingCode::parse("ABC"), Err(PairingCodeError::BadLength));
        assert_eq!(
            PairingCode::parse("ABCDE-FGHIU"),
            Err(PairingCodeError::BadCharacter)
        );
        assert_eq!(
            PairingCode::parse("ABCDE-FGH!J"),
            Err(PairingCodeError::BadCharacter)
        );
    }
}
