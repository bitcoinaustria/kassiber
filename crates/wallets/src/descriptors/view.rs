//! Non-SLIP-77 Liquid view keys (`ct(<key>, ...)`), read and derived as
//! Python does: embit's `BlindingKey` over a descriptor `Key`.
//!
//! embit's blinding key is the (derived) view key tweaked by
//! `tagged_hash("elements/blindingkey", script_pubkey)`, not ELIP-150's
//! tweak. Its "negate if odd" step discards the copy `ec_privkey_negate`
//! returns, so the key is never negated, and the public form agrees. Python
//! is the reference for stored data, so this module follows embit.
use super::{invalid_descriptor, Error, Result};
use bitcoin::bip32::{ChildNumber, Xpriv, Xpub};
use bitcoin::hashes::{sha256, Hash, HashEngine};
use bitcoin::secp256k1::{PublicKey, Scalar, Secp256k1, SecretKey};
use std::fmt;

const HARDENED: u32 = 1 << 31;

#[derive(Clone)]
pub(super) struct ViewKey {
    /// `[fingerprint/path]`, spelled as embit prints it.
    origin: Option<String>,
    material: Material,
    /// embit's `allowed_derivation`; empty when the key has none.
    steps: Vec<Step>,
}

#[derive(Clone)]
enum Material {
    /// A WIF private key (compressed or not) and its spelling.
    Secret { key: SecretKey, text: String },
    /// A public key and whether embit spells it compressed.
    Public { key: PublicKey, compressed: bool },
    /// An extended key, spelled as given (embit keeps its version bytes).
    Extended {
        xprv: Option<Xpriv>,
        xpub: Xpub,
        text: String,
    },
}

#[derive(Clone)]
enum Step {
    Index(u32),
    Set(Vec<u32>),
    Wildcard,
}

/// embit's base58 reader: the first invalid character, then the checksum,
/// both as Python `ValueError`s.
fn decode_check(text: &str) -> Result<Vec<u8>> {
    const DIGITS: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    if let Some(bad) = text.chars().find(|c| !DIGITS.contains(*c)) {
        return Err(Error::Value(format!(
            "Character {} is not a valid base58 character",
            kassiber_pytext::repr_str(&bad.to_string())
        )));
    }
    let raw = if text.is_empty() {
        vec![]
    } else {
        bitcoin::base58::decode(text).map_err(|_| invalid_descriptor())?
    };
    let split = raw.len().saturating_sub(4);
    let digest = sha256::Hash::hash(sha256::Hash::hash(&raw[..split]).as_ref());
    let checksum = &digest.as_byte_array()[..4];
    if &raw[split..] != checksum {
        return Err(Error::Value(format!(
            "Checksum mismatch: expected {}, calculated {}",
            kassiber_pyjson::bytes_repr(&raw[split..]),
            kassiber_pyjson::bytes_repr(checksum)
        )));
    }
    Ok(raw[..split].to_vec())
}

fn hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(Error::Value("Odd-length string".into()));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .ok_or_else(|| Error::Value("Non-hexadecimal digit found".into()))
        })
        .collect()
}

fn int(text: &str) -> Result<kassiber_pyjson::BigInt> {
    kassiber_pytext::parse_int(text).map_err(|_| {
        Error::Value(format!(
            "invalid literal for int() with base 10: {}",
            kassiber_pytext::repr_str(text)
        ))
    })
}

/// `KeyOrigin.from_string` then `str(origin)`.
fn origin(text: &str) -> Result<String> {
    let mut parts = text.split('/');
    let fingerprint = hex(parts.next().unwrap_or_default())?;
    if fingerprint.len() != 4 {
        return Err(invalid_descriptor());
    }
    let mut spelled: String = fingerprint.iter().map(|b| format!("{b:02x}")).collect();
    let path = parts.collect::<Vec<_>>().join("/");
    let path = path.trim_end_matches('/');
    if !path.is_empty() {
        let hardened = kassiber_pyjson::BigInt::from(HARDENED);
        for item in path.split('/') {
            let (digits, offset) = match item.strip_suffix(['h', 'H', '\'']) {
                Some(digits) => (digits, hardened.clone()),
                None if item.is_empty() => return Err(invalid_descriptor()),
                None => (item, 0.into()),
            };
            let value = int(digits)? + offset;
            if value >= hardened {
                spelled.push_str(&format!("/{}h", value - hardened.clone()));
            } else {
                spelled.push_str(&format!("/{value}"));
            }
        }
    }
    Ok(format!("[{spelled}]"))
}

/// `AllowedDerivation.parse_element`.
fn step(text: &str, allow_hardened: bool, allow_set: bool) -> Result<Step> {
    if text == "*" {
        return Ok(Step::Wildcard);
    }
    let set = (text.starts_with('<') && text.ends_with('>'))
        || (text.starts_with('{') && text.ends_with('}'));
    if set && text.len() >= 2 {
        if !allow_set {
            return Err(invalid_descriptor());
        }
        let separator = if text.starts_with('<') { ';' } else { ',' };
        let items = text[1..text.len() - 1]
            .split(separator)
            .map(|item| match step(item, allow_hardened, false)? {
                Step::Index(index) => Ok(index),
                _ => Err(invalid_descriptor()),
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(Step::Set(items));
    }
    if text.is_empty() {
        return Err(invalid_descriptor());
    }
    let (digits, hardened) = match text.strip_suffix(['h', 'H', '\'']) {
        Some(digits) => (digits, true),
        None => (text, false),
    };
    if hardened && !allow_hardened {
        return Err(invalid_descriptor());
    }
    let index = u32::try_from(int(digits)?)
        .ok()
        .filter(|i| *i < HARDENED)
        .ok_or_else(invalid_descriptor)?;
    Ok(Step::Index(if hardened { index + HARDENED } else { index }))
}

impl Material {
    /// `Key.parse_key`. An uncompressed key keeps its spelling, but embit
    /// blinds with the compressed point either way.
    fn parse(text: &str) -> Result<Self> {
        if matches!(text.len(), 66 | 130) && ["02", "03", "04"].iter().any(|p| text.starts_with(p))
        {
            let bytes = hex(text)?;
            let key = PublicKey::from_slice(&bytes).map_err(|_| invalid_descriptor())?;
            return Ok(Self::Public {
                key,
                compressed: bytes.len() == 33,
            });
        }
        let raw = decode_check(text)?;
        if matches!(text.get(1..4), Some("pub" | "prv")) {
            if raw.len() != 78 {
                return Err(invalid_descriptor());
            }
            let private = raw[45] == 0;
            let mut normalized = raw.clone();
            normalized[..4].copy_from_slice(if private {
                &[0x04, 0x88, 0xad, 0xe4]
            } else {
                &[0x04, 0x88, 0xb2, 0x1e]
            });
            let secp = Secp256k1::new();
            let (xprv, xpub) = if private {
                let xprv = Xpriv::decode(&normalized).map_err(|_| invalid_descriptor())?;
                (Some(xprv), Xpub::from_priv(&secp, &xprv))
            } else {
                (
                    None,
                    Xpub::decode(&normalized).map_err(|_| invalid_descriptor())?,
                )
            };
            return Ok(Self::Extended {
                xprv,
                xpub,
                text: text.into(),
            });
        }
        // `PrivateKey.from_wif`: a known prefix, then 32 bytes and an
        // optional compression flag.
        let flag_ok = match raw.len() {
            33 => true,
            34 => raw[33] == 0x01,
            _ => false,
        };
        if !flag_ok || !matches!(raw[0], 0x80 | 0xef) {
            return Err(invalid_descriptor());
        }
        let key = SecretKey::from_slice(&raw[1..33]).map_err(|_| invalid_descriptor())?;
        Ok(Self::Secret {
            key,
            text: text.into(),
        })
    }

    fn public(&self) -> PublicKey {
        match self {
            Self::Secret { key, .. } => key.public_key(&Secp256k1::new()),
            Self::Public { key, .. } => *key,
            Self::Extended { xpub, .. } => xpub.public_key,
        }
    }

    fn secret(&self) -> Option<SecretKey> {
        match self {
            Self::Secret { key, .. } => Some(*key),
            Self::Public { .. } => None,
            Self::Extended { xprv, .. } => xprv.map(|x| x.private_key),
        }
    }
}

/// `tagged_hash("elements/blindingkey", script_pubkey)`.
fn tweak(script: &[u8]) -> Result<Scalar> {
    let tag = sha256::Hash::hash(b"elements/blindingkey");
    let mut engine = sha256::Hash::engine();
    engine.input(tag.as_ref());
    engine.input(tag.as_ref());
    engine.input(script);
    Scalar::from_be_bytes(sha256::Hash::from_engine(engine).to_byte_array())
        .map_err(|_| Error::Value("Failed to tweak the secret".into()))
}

impl ViewKey {
    /// `Key.read_from` over the text between `blinded(` and the comma.
    pub(super) fn parse(text: &str) -> Result<Self> {
        let (origin, rest) = match text.strip_prefix('[') {
            Some(rest) => {
                let (spelled, rest) = rest.split_once(']').ok_or_else(invalid_descriptor)?;
                (Some(origin(spelled)?), rest)
            }
            None => (None, text),
        };
        let (key, derivation) = rest.split_once('/').unwrap_or((rest, ""));
        let material = Material::parse(key)?;
        let allow_hardened = matches!(material, Material::Extended { xprv: Some(_), .. });
        let steps = if derivation.is_empty() {
            vec![]
        } else {
            derivation
                .split('/')
                .map(|item| step(item, allow_hardened, true))
                .collect::<Result<Vec<_>>>()?
        };
        let count = |f: fn(&Step) -> bool| steps.iter().filter(|s| f(s)).count();
        if count(|s| matches!(s, Step::Wildcard)) > 1
            || count(|s| matches!(s, Step::Set(_))) > 1
            || (!steps.is_empty() && !matches!(material, Material::Extended { .. }))
        {
            return Err(invalid_descriptor());
        }
        Ok(Self {
            origin,
            material,
            steps,
        })
    }

    pub(super) fn is_wildcard(&self) -> bool {
        self.steps.iter().any(|s| matches!(s, Step::Wildcard))
    }

    /// `num_branches`: the branch set's size, else one.
    pub(super) fn branches(&self) -> usize {
        self.steps
            .iter()
            .find_map(|s| match s {
                Step::Set(items) => Some(items.len()),
                _ => None,
            })
            .unwrap_or(1)
    }

    pub(super) fn is_private(&self) -> bool {
        self.material.secret().is_some()
    }

    /// `Key.branch(index)`: the set replaced by its chosen element.
    pub(super) fn branch(&self, index: usize) -> Self {
        let mut branched = self.clone();
        for step in &mut branched.steps {
            if let Step::Set(items) = step {
                *step = Step::Index(items[index]);
            }
        }
        branched
    }

    /// `Key.derive(index)`: the wildcard filled, a set at its first element.
    fn derived(&self, index: u32) -> Result<(Option<SecretKey>, PublicKey)> {
        let Material::Extended { xprv, xpub, .. } = &self.material else {
            return Ok((self.material.secret(), self.material.public()));
        };
        if self.steps.is_empty() {
            return Ok((self.material.secret(), self.material.public()));
        }
        let path: Vec<ChildNumber> = self
            .steps
            .iter()
            .map(|s| match s {
                Step::Index(i) => ChildNumber::from(*i),
                Step::Set(items) => ChildNumber::from(items[0]),
                Step::Wildcard => ChildNumber::from(index),
            })
            .collect();
        let secp = Secp256k1::new();
        match xprv {
            Some(xprv) => {
                let child = xprv
                    .derive_priv(&secp, &path)
                    .map_err(|_| invalid_descriptor())?;
                Ok((Some(child.private_key), child.private_key.public_key(&secp)))
            }
            None => {
                let child = xpub
                    .derive_pub(&secp, &path)
                    .map_err(|_| invalid_descriptor())?;
                Ok((None, child.public_key))
            }
        }
    }

    /// The blinding public key of the target at `index` with `script`: what
    /// a confidential address carries.
    pub(super) fn blinding_public(&self, index: u32, script: &[u8]) -> Result<PublicKey> {
        let (_, public) = self.derived(index)?;
        public
            .add_exp_tweak(&Secp256k1::new(), &tweak(script)?)
            .map_err(|_| Error::Value("Failed to tweak the secret".into()))
    }

    /// `liquid_blinding_secret`'s key: the view key derived at `index`, as
    /// for the target's address. `None` for a public view key.
    pub(super) fn blinding_secret(&self, index: u32, script: &[u8]) -> Result<Option<SecretKey>> {
        let (secret, _) = self.derived(index)?;
        let Some(secret) = secret else {
            return Ok(None);
        };
        secret
            .add_tweak(&tweak(script)?)
            .map(Some)
            .map_err(|_| Error::Value("Failed to tweak the secret".into()))
    }
}

/// `str(BlindingKey)`: embit's key spelling.
impl fmt::Display for ViewKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(origin) = &self.origin {
            f.write_str(origin)?;
        }
        match &self.material {
            Material::Secret { text, .. } | Material::Extended { text, .. } => f.write_str(text)?,
            Material::Public { key, compressed } => {
                let bytes = if *compressed {
                    key.serialize().to_vec()
                } else {
                    key.serialize_uncompressed().to_vec()
                };
                for byte in bytes {
                    write!(f, "{byte:02x}")?;
                }
            }
        }
        let index = |i: u32| {
            if i >= HARDENED {
                format!("{}h", i - HARDENED)
            } else {
                i.to_string()
            }
        };
        for step in &self.steps {
            match step {
                Step::Index(i) => write!(f, "/{}", index(*i))?,
                Step::Set(items) => write!(
                    f,
                    "/<{}>",
                    items
                        .iter()
                        .map(|i| index(*i))
                        .collect::<Vec<_>>()
                        .join(";")
                )?,
                Step::Wildcard => f.write_str("/*")?,
            }
        }
        Ok(())
    }
}
