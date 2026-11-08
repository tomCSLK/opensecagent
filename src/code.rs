//! The enrollment code that opensecmon shows on the page of a computer (`docs/agent-api.md` of opensecmon,
//! "Enrollment code"): `osa1_` and, in Crockford's Base32, the address of opensecmon, the fingerprint of its
//! certificate and the one-time code, with a checksum against copying errors. The reference is
//! `src/agent_code.rs` in opensecmon; the example of the API document is a test here as well.

use sha2::{Digest, Sha256};

/// The prefix of an enrollment code (format version 1).
pub const PREFIX: &str = "osa1_";
/// Crockford's Base32 in lower case, as written.
const ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
/// The characters of a one-time code as opensecmon shows them.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Flag: a certificate fingerprint follows.
const WITH_FINGERPRINT: u8 = 1;
/// Longest address (host name or IP address with port).
pub const ADDRESS_MAX: usize = 255;
/// More characters than any enrollment code has; longer texts are refused before they are read.
const TEXT_MAX: usize = 1024;

/// What an enrollment code carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrollmentCode {
    /// Host name or IP address with an optional port, e.g. `192.0.2.10:8444`; always HTTPS.
    pub address: String,
    /// The SHA-256 fingerprint of the certificate of the built-in HTTPS.
    pub fingerprint: Option<[u8; 32]>,
    /// The one-time code as opensecmon shows it, e.g. `K7FQ-29XM`.
    pub code: String,
}

/// Why a text is no enrollment code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    /// Does not start with `osa1_` (another format, or no code at all).
    #[error("this is not an enrollment code: it starts with \"osa1_\"")]
    Prefix,
    /// Wrong characters, too short or too long, unknown flags or a checksum that does not match.
    #[error("the enrollment code is incomplete or damaged: copy it again from opensecmon")]
    Damaged,
}

/// Reads an enrollment code as copied: any case, spaces, line breaks and `-` are ignored.
pub fn decode(text: &str) -> Result<EnrollmentCode, CodeError> {
    let text = text.trim();
    let rest = text
        .get(..PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(PREFIX))
        .map(|_| &text[PREFIX.len()..])
        .ok_or(CodeError::Prefix)?;
    if rest.len() > TEXT_MAX {
        return Err(CodeError::Damaged);
    }
    let payload = unbase32(rest).ok_or(CodeError::Damaged)?;
    // Flags, the code, at least one character of the address and the checksum.
    if payload.len() < 1 + 5 + 1 + 4 {
        return Err(CodeError::Damaged);
    }
    let (body, sum) = payload.split_at(payload.len() - 4);
    if Sha256::digest(body)[..4] != *sum {
        return Err(CodeError::Damaged);
    }
    let flags = body[0];
    if flags & !WITH_FINGERPRINT != 0 {
        return Err(CodeError::Damaged);
    }
    let mut rest = &body[1..];
    let fingerprint = if flags & WITH_FINGERPRINT != 0 {
        let (fp, after) = rest.split_at_checked(32).ok_or(CodeError::Damaged)?;
        rest = after;
        Some(<[u8; 32]>::try_from(fp).map_err(|_| CodeError::Damaged)?)
    } else {
        None
    };
    let (code, address) = rest.split_at_checked(5).ok_or(CodeError::Damaged)?;
    let address = std::str::from_utf8(address).map_err(|_| CodeError::Damaged)?;
    if !is_address(address) {
        return Err(CodeError::Damaged);
    }
    Ok(EnrollmentCode {
        address: address.to_owned(),
        fingerprint,
        code: bytes_to_code(code.try_into().map_err(|_| CodeError::Damaged)?),
    })
}

/// Whether a text can be the address of an enrollment code: a host name or IP address with an optional port, no
/// path, no user, no scheme.
pub fn is_address(address: &str) -> bool {
    !address.is_empty()
        && address.len() <= ADDRESS_MAX
        && address
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
}

/// A fingerprint as certificates show it: `AB:CD:…`.
pub fn format_fingerprint(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// A fingerprint as [`format_fingerprint`] writes it, back as bytes.
pub fn parse_fingerprint(text: &str) -> Option<[u8; 32]> {
    let hex: String = text.chars().filter(|c| *c != ':').collect();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// 40 bits as the one-time code is shown (`K7FQ-29XM`).
fn bytes_to_code(bytes: [u8; 5]) -> String {
    let bits = bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
    let chars: String = (0..8)
        .rev()
        .map(|i| char::from(CODE_ALPHABET[((bits >> (5 * i)) & 31) as usize]))
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

/// Reads Crockford's Base32 in any case; spaces, line breaks and `-` are skipped, `o` reads as `0`, `i` and `l`
/// as `1`. `None` for other characters or bits left over that are not zero.
fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 5 / 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.chars() {
        if c.is_whitespace() || c == '-' {
            continue;
        }
        let c = match c.to_ascii_lowercase() {
            'o' => '0',
            'i' | 'l' => '1',
            c => c,
        };
        let value = ALPHABET.iter().position(|a| char::from(*a) == c)? as u32;
        buffer = ((buffer << 5) | value) & 0xffff;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    (buffer & ((1 << bits) - 1) == 0).then_some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Crockford's Base32 as opensecmon writes it (only for building codes in the tests).
    pub(crate) fn base32(bytes: &[u8]) -> String {
        let mut out = String::new();
        let (mut buffer, mut bits) = (0u32, 0u32);
        for &b in bytes {
            buffer = (buffer << 8) | u32::from(b);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
            }
        }
        if bits > 0 {
            out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
        }
        out
    }

    /// An enrollment code as opensecmon writes it (`agent_code::encode`).
    pub(crate) fn encode(address: &str, fingerprint: Option<&[u8; 32]>, code: &str) -> String {
        let plain: Vec<u8> = code.bytes().filter(|b| *b != b'-').collect();
        assert_eq!(plain.len(), 8);
        let bits = plain.iter().fold(0u64, |acc, b| {
            let value = CODE_ALPHABET.iter().position(|c| c == b).unwrap() as u64;
            (acc << 5) | value
        });
        let mut payload = vec![u8::from(fingerprint.is_some())];
        if let Some(fp) = fingerprint {
            payload.extend_from_slice(fp);
        }
        payload.extend_from_slice(&bits.to_be_bytes()[3..]);
        payload.extend_from_slice(address.as_bytes());
        let sum = Sha256::digest(&payload);
        payload.extend_from_slice(&sum[..4]);
        format!("{PREFIX}{}", base32(&payload))
    }

    /// The example of `docs/agent-api.md` in opensecmon.
    const EXAMPLE: &str = "osa1_040020g30g2gc1r81450p30d1r7h048j2ca1a5gq30chm6rw3mf1z6eze4kv8c9s68q30bhj5rrk0ehr6gt38epxvbd0";

    #[test]
    fn the_example_of_the_api_document() {
        let fp: [u8; 32] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            decode(EXAMPLE).unwrap(),
            EnrollmentCode {
                address: "192.0.2.10:8444".to_owned(),
                fingerprint: Some(fp),
                code: "K7FQ-29XM".to_owned(),
            }
        );
        assert_eq!(encode("192.0.2.10:8444", Some(&fp), "K7FQ-29XM"), EXAMPLE);
        assert_eq!(EXAMPLE.len(), 97, "as the API document says");
    }

    #[test]
    fn codes_with_and_without_fingerprint() {
        for (address, fp) in [
            ("192.0.2.10:8444", Some([0xab; 32])),
            ("[fd00::2]:8444", Some([0; 32])),
            ("secmon.example.org", None),
            ("a", None),
        ] {
            for code in ["K7FQ-29XM", "0000-0000", "ZZZZ-ZZZZ"] {
                let back = decode(&encode(address, fp.as_ref(), code)).unwrap();
                assert_eq!(back.address, address);
                assert_eq!(back.fingerprint, fp);
                assert_eq!(back.code, code);
            }
        }
    }

    #[test]
    fn a_code_survives_how_people_copy_it() {
        let expected = decode(EXAMPLE).unwrap();
        assert_eq!(decode(&EXAMPLE.to_ascii_uppercase()).unwrap(), expected);
        let (a, b) = EXAMPLE.split_at(40);
        assert_eq!(decode(&format!("  {a}\r\n  {b} \n")).unwrap(), expected);
        assert_eq!(decode(&format!("{a}-{b}")).unwrap(), expected);
        // Letters people mix up with digits.
        let payload = &EXAMPLE[PREFIX.len()..];
        let mixed = format!(
            "{PREFIX}{}",
            payload.replacen('0', "o", 3).replacen('1', "l", 2)
        );
        assert_ne!(mixed, EXAMPLE);
        assert_eq!(decode(&mixed).unwrap(), expected);
    }

    #[test]
    fn damaged_codes_are_refused() {
        assert_eq!(decode("K7FQ-29XM"), Err(CodeError::Prefix));
        assert_eq!(decode(""), Err(CodeError::Prefix));
        assert_eq!(decode("osa2_abc"), Err(CodeError::Prefix));
        assert_eq!(decode("osä1_abc"), Err(CodeError::Prefix));
        assert_eq!(
            decode(&EXAMPLE[..EXAMPLE.len() - 1]),
            Err(CodeError::Damaged)
        );
        assert_eq!(decode(&EXAMPLE[..60]), Err(CodeError::Damaged));
        assert_eq!(decode(PREFIX), Err(CodeError::Damaged));
        for i in PREFIX.len()..EXAMPLE.len() {
            let mut changed = EXAMPLE.to_owned().into_bytes();
            changed[i] = if changed[i] == b'x' { b'y' } else { b'x' };
            let changed = String::from_utf8(changed).unwrap();
            assert_eq!(decode(&changed), Err(CodeError::Damaged), "position {i}");
            let mut missing = EXAMPLE.to_owned();
            missing.remove(i);
            assert_eq!(decode(&missing), Err(CodeError::Damaged), "position {i}");
        }
        assert_eq!(decode(&format!("{EXAMPLE}0")), Err(CodeError::Damaged));
        assert_eq!(decode(&format!("{EXAMPLE}u")), Err(CodeError::Damaged));
        assert_eq!(decode(&format!("{EXAMPLE}ä")), Err(CodeError::Damaged));
        assert_eq!(
            decode(&format!("{PREFIX}{}", "0".repeat(5000))),
            Err(CodeError::Damaged)
        );
    }

    #[test]
    fn unknown_flags_and_bad_addresses_are_refused() {
        let with = |flags: u8, address: &[u8]| {
            let mut payload = vec![flags];
            payload.extend_from_slice(&[0; 5]);
            payload.extend_from_slice(address);
            let sum = Sha256::digest(&payload);
            payload.extend_from_slice(&sum[..4]);
            decode(&format!("{PREFIX}{}", base32(&payload)))
        };
        assert!(with(0, b"host").is_ok());
        assert_eq!(with(2, b"host"), Err(CodeError::Damaged));
        assert_eq!(with(0, b"host/$(reboot)"), Err(CodeError::Damaged));
        assert_eq!(with(0, b"user@host"), Err(CodeError::Damaged));
        assert_eq!(with(0, b"h\xc3\xb6st"), Err(CodeError::Damaged));
        assert_eq!(with(0, &[b'a'; 256]), Err(CodeError::Damaged));
        // Flag set, but no room for the fingerprint.
        assert_eq!(with(1, b"host"), Err(CodeError::Damaged));
    }

    #[test]
    fn fingerprints_are_written_and_read_as_certificates_show_them() {
        let fp: [u8; 32] = std::array::from_fn(|i| (i * 7) as u8);
        let text = format_fingerprint(&fp);
        assert_eq!(&text[..9], "00:07:0E:");
        assert_eq!(text.len(), 95);
        assert_eq!(parse_fingerprint(&text), Some(fp));
        assert_eq!(parse_fingerprint(&text.to_ascii_lowercase()), Some(fp));
        assert_eq!(parse_fingerprint("AB:CD"), None);
        assert_eq!(parse_fingerprint(&text.replace("0E", "0G")), None);
        assert_eq!(parse_fingerprint(&"é".repeat(32)), None);
    }

    #[test]
    fn base32_reads_what_it_writes() {
        for len in 0..40 {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(unbase32(&base32(&bytes)).unwrap(), bytes, "{len}");
        }
        assert_eq!(unbase32("cr").unwrap(), b"f");
        assert_eq!(unbase32("cs"), None, "bits left over must be zero");
    }
}
