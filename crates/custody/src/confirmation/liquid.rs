//! The embit 0.8 parse-success boundary used by saved confirmation capture.
//!
//! This is deliberately not consensus validation or ownership decoding. Python
//! discards the decoded txid here and uses only whether a partial public graph
//! exists. In particular, embit permits short locktimes, noncanonical compact
//! integers and arbitrary commitment prefixes. Preserve that boundary without
//! promoting any decoded value, script or asset into accounting evidence.

struct Reader<'a> {
    rest: &'a [u8],
}
impl Reader<'_> {
    // BytesIO.read returns the remaining bytes on a short read. Its length
    // argument must fit Py_ssize_t, even when the stream has fewer bytes.
    fn read(&mut self, count: u64) -> Option<&[u8]> {
        let count = usize::try_from(count).ok()?;
        if count > isize::MAX as usize {
            return None;
        }
        let end = count.min(self.rest.len());
        let (data, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some(data)
    }
    fn byte(&mut self) -> Option<u8> {
        self.read(1)?.first().copied()
    }
    fn integer(&mut self, size: u64) -> Option<u64> {
        let mut bytes = [0; 8];
        let data = self.read(size)?;
        bytes[..data.len()].copy_from_slice(data);
        Some(u64::from_le_bytes(bytes))
    }
    fn compact(&mut self) -> Option<u64> {
        match self.byte()? {
            marker @ 0xfd..=0xff => self.integer(1 << (marker - 0xfc)),
            value => Some(value.into()),
        }
    }
    fn exact(&mut self, count: u64) -> Option<()> {
        (self.read(count)?.len() as u64 == count).then_some(())
    }
    fn script(&mut self) -> Option<()> {
        let count = self.compact()?;
        self.exact(count)
    }
    fn proof(&mut self) -> Option<()> {
        let count = self.compact()?;
        self.read(count)?;
        Some(())
    }
    fn witness(&mut self) -> Option<()> {
        let count = self.count()?;
        for _ in 0..count {
            self.proof()?;
        }
        Some(())
    }
    fn commitment(&mut self) -> Option<()> {
        match self.byte()? {
            0 => Some(()),
            1 => self.exact(8),
            _ => self.exact(32),
        }
    }
    // Every successful item consumes at least one byte. This bound avoids
    // iterating attacker-selected counts without narrowing embit acceptance.
    fn count(&mut self) -> Option<u64> {
        let count = self.compact()?;
        (count <= self.rest.len() as u64).then_some(count)
    }
    fn transaction(&mut self) -> Option<()> {
        self.read(4)?;
        let witness = self.byte()? == 1;
        let inputs = self.count()?;
        for _ in 0..inputs {
            self.read(32)?;
            let vout = self.integer(4)?;
            self.script()?;
            self.read(4)?;
            if vout != 0xffff_ffff && vout & (1 << 31) != 0 {
                self.exact(32)?;
                self.exact(32)?;
                self.commitment()?;
                self.commitment()?;
            }
        }
        let outputs = self.count()?;
        for _ in 0..outputs {
            self.read(33)?;
            if self.byte()? == 1 {
                self.read(8)?;
            } else {
                self.read(32)?;
            }
            if self.byte()? != 0 {
                self.read(32)?;
            }
            self.script()?;
        }
        self.read(4)?;
        if witness {
            for _ in 0..inputs {
                self.proof()?;
                self.proof()?;
                self.witness()?;
                self.witness()?;
            }
            for _ in 0..outputs {
                self.proof()?;
                self.proof()?;
            }
        }
        self.rest.is_empty().then_some(())
    }
}

pub(super) fn decodable(hex: &str) -> bool {
    let hex = crate::python_text::strip(hex).as_bytes();
    if hex.is_empty() || !hex.len().is_multiple_of(2) {
        return false;
    }
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    let bytes: Option<Vec<_>> = hex
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| Some(digit(pair[0])? * 16 + digit(pair[1])?))
        .collect();
    bytes.is_some_and(|bytes| Reader { rest: &bytes }.transaction().is_some())
}
