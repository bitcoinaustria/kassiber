//! Narrow X.509 compatibility checks for byte-identical backend pins only.
//! rustls/webpki parses the entire certificate first. This DER reader extracts
//! validity, DNS SAN presence, EKU and subject CN; it never validates a chain.
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, Error};

fn invalid() -> Error {
    Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)
}

fn take<'a>(input: &mut &'a [u8]) -> Result<(u8, &'a [u8]), Error> {
    let (&tag, rest) = input.split_first().ok_or_else(invalid)?;
    let (&first, mut rest) = rest.split_first().ok_or_else(invalid)?;
    let length = if first < 128 {
        usize::from(first)
    } else {
        let count = usize::from(first & 127);
        if count == 0 || count > 4 || rest.first() == Some(&0) {
            return Err(invalid());
        }
        let bytes = rest.get(..count).ok_or_else(invalid)?;
        let length = bytes
            .iter()
            .fold(0usize, |n, byte| (n << 8) | usize::from(*byte));
        if length < 128 {
            return Err(invalid());
        }
        rest = &rest[count..];
        length
    };
    let value = rest.get(..length).ok_or_else(invalid)?;
    *input = &rest[length..];
    Ok((tag, value))
}

fn tagged<'a>(input: &mut &'a [u8], expected: u8) -> Result<&'a [u8], Error> {
    let (tag, value) = take(input)?;
    if tag != expected {
        return Err(invalid());
    }
    Ok(value)
}

fn sequence(input: &[u8]) -> Result<&[u8], Error> {
    let mut input = input;
    let value = tagged(&mut input, 0x30)?;
    if !input.is_empty() {
        return Err(invalid());
    }
    Ok(value)
}

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn timestamp(tag: u8, value: &[u8]) -> Result<i64, Error> {
    let digits = match (tag, value.len()) {
        (0x17, 13) => 2,
        (0x18, 15) => 4,
        _ => return Err(invalid()),
    };
    if value.last() != Some(&b'Z') || !value[..value.len() - 1].iter().all(u8::is_ascii_digit) {
        return Err(invalid());
    }
    let number = |start: usize, len: usize| {
        value[start..start + len]
            .iter()
            .fold(0i64, |n, b| n * 10 + i64::from(b - b'0'))
    };
    let mut year = number(0, digits);
    if digits == 2 {
        year += if year >= 50 { 1900 } else { 2000 };
    }
    let month = number(digits, 2);
    let day = number(digits + 2, 2);
    let hour = number(digits + 4, 2);
    let minute = number(digits + 6, 2);
    let second = number(digits + 8, 2);
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || !(1..=months[(month - 1) as usize]).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(invalid());
    }
    let mut days = if year >= 1970 {
        (1970..year)
            .map(|y| if leap(y) { 366 } else { 365 })
            .sum::<i64>()
    } else {
        -(year..1970)
            .map(|y| if leap(y) { 366 } else { 365 })
            .sum::<i64>()
    };
    days += months[..(month - 1) as usize].iter().sum::<i64>() + day - 1;
    Ok(((days * 24 + hour) * 60 + minute) * 60 + second)
}

fn dns_match(pattern: &str, name: &str) -> bool {
    if !pattern.is_ascii() || pattern.contains('\0') {
        return false;
    }
    if !pattern.contains('*') {
        return pattern.eq_ignore_ascii_case(name);
    }
    // Python/OpenSSL disables partial wildcards and permits one leftmost label.
    let Some(suffix) = pattern.strip_prefix("*.") else {
        return false;
    };
    let Some((label, rest)) = name.split_once('.') else {
        return false;
    };
    !label.is_empty()
        && !suffix.contains('*')
        && suffix.contains('.')
        && suffix.eq_ignore_ascii_case(rest)
}

pub(super) fn verify(
    cert: &CertificateDer<'_>,
    name: &ServerName<'_>,
    now: UnixTime,
) -> Result<(), Error> {
    let parsed = rustls::server::ParsedCertificate::try_from(cert)?;
    let mut certificate = sequence(cert.as_ref())?;
    let mut tbs = tagged(&mut certificate, 0x30)?;
    if tbs.first() == Some(&0xa0) {
        tagged(&mut tbs, 0xa0)?;
    }
    tagged(&mut tbs, 0x02)?; // serial
    tagged(&mut tbs, 0x30)?; // signature algorithm
    tagged(&mut tbs, 0x30)?; // issuer
    let mut validity = tagged(&mut tbs, 0x30)?;
    let (tag, value) = take(&mut validity)?;
    let before = timestamp(tag, value)?;
    let (tag, value) = take(&mut validity)?;
    let after = timestamp(tag, value)?;
    let now = i64::try_from(now.as_secs()).map_err(|_| invalid())?;
    if !validity.is_empty() || before > after || now < before || now > after {
        return Err(invalid());
    }
    let mut subject = tagged(&mut tbs, 0x30)?;
    tagged(&mut tbs, 0x30)?; // SPKI
    let mut dns_san = false;
    while !tbs.is_empty() {
        let (tag, value) = take(&mut tbs)?;
        if tag != 0xa3 {
            return Err(invalid());
        }
        let mut extensions = sequence(value)?;
        while !extensions.is_empty() {
            let mut extension = tagged(&mut extensions, 0x30)?;
            let oid = tagged(&mut extension, 0x06)?;
            if extension.first() == Some(&0x01) {
                tagged(&mut extension, 0x01)?;
            }
            let value = tagged(&mut extension, 0x04)?;
            if !extension.is_empty() {
                return Err(invalid());
            }
            if oid == [0x55, 0x1d, 0x11] {
                let mut names = sequence(value)?;
                while !names.is_empty() {
                    dns_san |= take(&mut names)?.0 == 0x82;
                }
            } else if oid == [0x55, 0x1d, 0x25] {
                let mut usages = sequence(value)?;
                let mut server = false;
                while !usages.is_empty() {
                    let usage = tagged(&mut usages, 0x06)?;
                    server |=
                        usage == [0x2b, 6, 1, 5, 5, 7, 3, 1] || usage == [0x55, 0x1d, 0x25, 0];
                }
                if !server {
                    return Err(invalid());
                }
            }
        }
    }
    if rustls::client::verify_server_name(&parsed, name).is_ok() {
        return Ok(());
    }
    // IP reference identities require IP SANs; never compare an IP with a CN.
    let ServerName::DnsName(name) = name else {
        return Err(invalid());
    };
    if dns_san {
        return Err(invalid());
    }
    while !subject.is_empty() {
        let mut rdn = tagged(&mut subject, 0x31)?;
        while !rdn.is_empty() {
            let mut attribute = tagged(&mut rdn, 0x30)?;
            let oid = tagged(&mut attribute, 0x06)?;
            let (tag, value) = take(&mut attribute)?;
            if !attribute.is_empty() {
                return Err(invalid());
            }
            if oid != [0x55, 4, 3] {
                continue;
            }
            let value = match tag {
                0x0c | 0x13 | 0x14 | 0x16 => std::str::from_utf8(value)
                    .map(str::to_owned)
                    .map_err(|_| invalid())?,
                0x1e if value.len() % 2 == 0 => {
                    if value
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .any(|pair| pair[0] != 0 || !pair[1].is_ascii())
                    {
                        return Err(invalid());
                    }
                    value
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| char::from(pair[1]))
                        .collect()
                }
                _ => return Err(invalid()),
            };
            if dns_match(&value, name.as_ref()) {
                return Ok(());
            }
        }
    }
    Err(invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cn_wildcards_and_der_times_fail_closed() {
        assert!(dns_match("*.example.com", "node.example.com"));
        for (pattern, name) in [
            ("*.example.com", "a.b.example.com"),
            ("n*.example.com", "node.example.com"),
            ("*.*.com", "a.b.com"),
            ("localhost\0evil", "localhost"),
        ] {
            assert!(!dns_match(pattern, name));
        }
        assert_eq!(timestamp(0x17, b"700101000000Z").unwrap(), 0);
        assert_eq!(timestamp(0x18, b"20000229000000Z").unwrap(), 951782400);
        for value in [
            b"230229000000Z",
            b"240230000000Z",
            b"240101240000Z",
            b"240101000060Z",
        ] {
            assert!(timestamp(0x17, value).is_err());
        }
        for input in [
            &b"\x30\x80"[..],
            &b"\x30\x81\x01x"[..],
            &b"\x30\x82\xff"[..],
        ] {
            assert!(sequence(input).is_err());
        }
    }
}
