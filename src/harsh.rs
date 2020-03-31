use crate::error::{Error, Result};
use std::str;

const DEFAULT_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890";
const DEFAULT_SEPARATORS: &[u8] = b"cfhistuCFHISTU";
const SEPARATOR_DIV: f64 = 3.5;
const GUARD_DIV: f64 = 12.0;
const MINIMUM_ALPHABET_LENGTH: usize = 16;

/// A hashids-compatible hasher.
///
/// It's probably not a great idea to use the default, because in that case
/// your values will be entirely trivial to decode. On the other hand, this is
/// not intended to be cryptographically-secure, so go nuts!
#[derive(Clone, Debug)]
pub struct Harsh {
    salt: Box<[u8]>,
    alphabet: Box<[u8]>,
    separators: Box<[u8]>,
    hash_length: usize,
    guards: Box<[u8]>,
}

impl Harsh {
    /// Encodes a slice of `u64` values into a single hashid.
    pub fn encode(&self, values: &[u64]) -> Option<String> {
        if values.is_empty() {
            return None;
        }

        let nhash = create_nhash(values);

        let mut alphabet = self.alphabet.clone();
        let mut buffer = String::new();

        let idx = (nhash % alphabet.len() as u64) as usize;
        let lottery = alphabet[idx];
        buffer.push(lottery as char);

        for (idx, &value) in values.iter().enumerate() {
            let mut value = value;

            let temp = {
                let mut temp = Vec::with_capacity(self.salt.len() + alphabet.len() + 1);
                temp.push(lottery);
                temp.extend_from_slice(&self.salt);
                temp.extend_from_slice(&alphabet);
                temp
            };

            let alphabet_len = alphabet.len();
            shuffle(&mut alphabet, &temp[..alphabet_len]);

            let last = hash(value, &alphabet);
            buffer.push_str(&last);

            if idx + 1 < values.len() {
                value %= (last.bytes().nth(0).unwrap_or(0) as usize + idx) as u64;
                buffer
                    .push(self.separators[(value % self.separators.len() as u64) as usize] as char);
            }
        }

        if buffer.len() < self.hash_length {
            let guard_index = (nhash as usize
                + buffer.bytes().nth(0).expect("hellfire and damnation") as usize)
                % self.guards.len();
            let guard = self.guards[guard_index];
            buffer.insert(0, guard as char);

            if buffer.len() < self.hash_length {
                let guard_index = (nhash as usize
                    + buffer.bytes().nth(2).expect("hellfire and damnation") as usize)
                    % self.guards.len();
                let guard = self.guards[guard_index];
                buffer.push(guard as char);
            }
        }

        let half_length = alphabet.len() / 2;
        while buffer.len() < self.hash_length {
            {
                let alphabet_copy = alphabet.clone();
                shuffle(&mut alphabet, &alphabet_copy);
            }

            let (left, right) = alphabet.split_at(half_length);
            buffer = format!(
                "{}{}{}",
                String::from_utf8_lossy(right),
                buffer,
                String::from_utf8_lossy(left)
            );

            let excess = buffer.len() as i32 - self.hash_length as i32;
            if excess > 0 {
                let marker = excess as usize / 2;
                buffer = buffer[marker..marker + self.hash_length].to_owned();
            }
        }

        Some(buffer)
    }

    /// Decodes a single hashid into a slice of `u64` values.
    pub fn decode<T: AsRef<str>>(&self, value: T) -> Option<Vec<u64>> {
        let mut value = value.as_ref().as_bytes();

        if let Some(guard_idx) = value.iter().position(|u| self.guards.contains(u)) {
            value = &value[(guard_idx + 1)..];
        }

        if let Some(guard_idx) = value.iter().rposition(|u| self.guards.contains(u)) {
            value = &value[..guard_idx];
        }

        if value.len() < 2 {
            return None;
        }

        let mut alphabet = self.alphabet.clone();

        let lottery = value[0];
        let value = &value[1..];
        let segments: Vec<_> = value.split(|u| self.separators.contains(u)).collect();

        segments
            .into_iter()
            .map(|segment| {
                let mut buffer = Vec::with_capacity(self.salt.len() + alphabet.len() + 1);
                buffer.push(lottery);
                buffer.extend_from_slice(&self.salt);
                buffer.extend_from_slice(&alphabet);

                let alphabet_len = alphabet.len();
                shuffle(&mut alphabet, &buffer[..alphabet_len]);
                unhash(segment, &alphabet)
            })
            .collect()
    }

    /// Encodes a hex string into a hashid.
    pub fn encode_hex(&self, hex: &str) -> Option<String> {
        let values: Option<Vec<_>> = hex
            .as_bytes()
            .chunks(12)
            .map(|chunk| {
                str::from_utf8(chunk)
                    .ok()
                    .and_then(|s| u64::from_str_radix(&("1".to_owned() + s), 16).ok())
            })
            .collect();

        values.and_then(|values| self.encode(&values))
    }

    /// Decodes a hashid into a hex string.
    pub fn decode_hex(&self, value: &str) -> Option<String> {
        use std::fmt::Write;

        match self.decode(value) {
            None => None,
            Some(ref values) => {
                let mut result = String::new();
                let mut buffer = String::new();

                for n in values {
                    write!(buffer, "{:x}", n).expect("failed to write?!");
                    result.push_str(&buffer[1..]);
                    buffer.clear();
                }

                Some(result)
            }
        }
    }
}

impl Default for Harsh {
    fn default() -> Harsh {
        HarshBuilder::new().init().unwrap()
    }
}

/// A builder used to configure and create a Harsh instance.
#[derive(Debug, Default)]
pub struct HarshBuilder {
    salt: Option<Vec<u8>>,
    alphabet: Option<Vec<u8>>,
    separators: Option<Vec<u8>>,
    hash_length: usize,
}

impl HarshBuilder {
    /// Creates a new `HarshBuilder` instance.
    pub fn new() -> HarshBuilder {
        HarshBuilder {
            salt: None,
            alphabet: None,
            separators: None,
            hash_length: 0,
        }
    }

    /// Provides a salt.
    ///
    /// Note that this salt will be converted into a `[u8]` before use, meaning
    /// that multi-byte utf8 character values should be avoided.
    pub fn salt<T: Into<Vec<u8>>>(mut self, salt: T) -> HarshBuilder {
        self.salt = Some(salt.into());
        self
    }

    /// Provides an alphabet.
    ///
    /// Note that this alphabet will be converted into a `[u8]` before use, meaning
    /// that multi-byte utf8 character values should be avoided.
    pub fn alphabet<T: Into<Vec<u8>>>(mut self, alphabet: T) -> HarshBuilder {
        self.alphabet = Some(alphabet.into());
        self
    }

    /// Provides a set of separators.
    ///
    /// Note that these separators will be converted into a `[u8]` before use,
    /// meaning that multi-byte utf8 character values should be avoided.
    pub fn separators<T: Into<Vec<u8>>>(mut self, separators: T) -> HarshBuilder {
        self.separators = Some(separators.into());
        self
    }

    /// Provides a minimum hash length.
    ///
    /// Keep in mind that hashes produced may be longer than this length.
    pub fn length(mut self, hash_length: usize) -> HarshBuilder {
        self.hash_length = hash_length;
        self
    }

    /// Initializes a new `Harsh` based on the `HarshBuilder`.
    ///
    /// This method will consume the `HarshBuilder`.
    pub fn init(self) -> Result<Harsh> {
        let alphabet = unique_alphabet(&self.alphabet)?;
        if alphabet.len() < MINIMUM_ALPHABET_LENGTH {
            return Err(Error::AlphabetLength);
        }

        let salt = self.salt.unwrap_or_else(Vec::new);
        let (mut alphabet, mut separators) =
            alphabet_and_separators(&self.separators, &alphabet, &salt);
        let guards = guards(&mut alphabet, &mut separators);

        Ok(Harsh {
            salt: salt.into_boxed_slice(),
            alphabet: alphabet.into_boxed_slice(),
            separators: separators.into_boxed_slice(),
            hash_length: self.hash_length,
            guards: guards.into_boxed_slice(),
        })
    }
}

#[inline]
fn create_nhash(values: &[u64]) -> u64 {
    values
        .iter()
        .enumerate()
        .fold(0, |a, (idx, value)| a + (value % (idx + 100) as u64))
}

fn unique_alphabet(alphabet: &Option<Vec<u8>>) -> Result<Vec<u8>> {
    use std::collections::HashSet;

    match *alphabet {
        None => {
            let mut vec = vec![0; DEFAULT_ALPHABET.len()];
            vec.clone_from_slice(DEFAULT_ALPHABET);
            Ok(vec)
        }

        Some(ref alphabet) => {
            let mut reg = HashSet::new();
            let mut ret = Vec::new();

            for &item in alphabet {
                if item == b' ' {
                    return Err(Error::IllegalCharacter(item as char));
                }

                if !reg.contains(&item) {
                    ret.push(item);
                    reg.insert(item);
                }
            }

            if ret.len() < 16 {
                Err(Error::AlphabetLength)
            } else {
                Ok(ret)
            }
        }
    }
}

fn alphabet_and_separators(
    separators: &Option<Vec<u8>>,
    alphabet: &[u8],
    salt: &[u8],
) -> (Vec<u8>, Vec<u8>) {
    let separators = match *separators {
        None => DEFAULT_SEPARATORS,
        Some(ref separators) => separators,
    };

    let mut separators: Vec<_> = separators
        .iter()
        .cloned()
        .filter(|item| alphabet.contains(item))
        .collect();
    let mut alphabet: Vec<_> = alphabet
        .iter()
        .cloned()
        .filter(|item| !separators.contains(item))
        .collect();

    shuffle(&mut separators, salt);

    if separators.is_empty() || (alphabet.len() as f64 / separators.len() as f64) > SEPARATOR_DIV {
        let length = match (alphabet.len() as f64 / SEPARATOR_DIV).ceil() as usize {
            1 => 2,
            n => n,
        };

        if length > separators.len() {
            let diff = length - separators.len();
            separators.extend_from_slice(&alphabet[..diff]);
            alphabet = alphabet[diff..].to_vec();
        } else {
            separators = separators[..length].to_vec();
        }
    }

    shuffle(&mut alphabet, salt);
    (alphabet, separators)
}

fn guards(alphabet: &mut Vec<u8>, separators: &mut Vec<u8>) -> Vec<u8> {
    let guard_count = (alphabet.len() as f64 / GUARD_DIV).ceil() as usize;
    if alphabet.len() < 3 {
        let guards = separators[..guard_count].to_vec();
        separators.drain(..guard_count);
        guards
    } else {
        let guards = alphabet[..guard_count].to_vec();
        alphabet.drain(..guard_count);
        guards
    }
}

fn shuffle(values: &mut [u8], salt: &[u8]) {
    if salt.is_empty() {
        return;
    }

    let values_length = values.len();
    let salt_length = salt.len();
    let (mut v, mut p) = (0, 0);

    for i in (1..values_length).map(|i| values_length - i) {
        v %= salt_length;

        let n = salt[v] as usize;
        p += n;
        let j = (n + v + p) % i;

        values.swap(i, j);
        v += 1;
    }
}

fn hash(mut value: u64, alphabet: &[u8]) -> String {
    let length = alphabet.len() as u64;
    let mut hash = Vec::new();

    loop {
        hash.push(alphabet[(value % length) as usize]);
        value /= length;

        if value == 0 {
            hash.reverse();
            return String::from_utf8(hash).expect("omg fml");
        }
    }
}

fn unhash(input: &[u8], alphabet: &[u8]) -> Option<u64> {
    input.iter().enumerate().fold(Some(0), |a, (idx, &value)| {
        let pos = alphabet.iter().position(|&item| item == value)? as u64;
        a.map(|a| a + (pos * (alphabet.len() as u64).pow((input.len() - idx - 1) as u32)))
    })
}

#[cfg(test)]
mod tests {
    use super::{Harsh, HarshBuilder};

    #[test]
    fn harsh_default_does_not_panic() {
        Harsh::default();
    }

    #[test]
    fn can_encode() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "4o6Z7KqxE",
            harsh.encode(&[1226198605112]).expect("failed to encode"),
            "error encoding [1226198605112]"
        );
        assert_eq!(
            "laHquq",
            harsh.encode(&[1, 2, 3]).expect("failed to encode")
        );
    }

    #[test]
    fn can_encode_with_guards() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(8)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "GlaHquq0",
            harsh.encode(&[1, 2, 3]).expect("failed to encode")
        );
    }

    #[test]
    fn can_encode_with_padding() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(12)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "9LGlaHquq06D",
            harsh.encode(&[1, 2, 3]).expect("failed to encode")
        );
    }

    #[test]
    fn can_decode() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            &[1226198605112],
            &harsh.decode("4o6Z7KqxE").expect("failed to decode")[..],
            "error decoding \"4o6Z7KqxE\""
        );
        assert_eq!(
            &[1u64, 2, 3],
            &harsh.decode("laHquq").expect("failed to decode")[..]
        );
    }

    #[test]
    fn can_decode_with_guards() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(8)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            &[1u64, 2, 3],
            &harsh.decode("GlaHquq0").expect("failed to decode")[..]
        );
    }

    #[test]
    fn can_decode_with_padding() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(12)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            &[1u64, 2, 3],
            &harsh.decode("9LGlaHquq06D").expect("failed to decode")[..]
        );
    }

    #[test]
    fn can_encode_hex() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "lzY",
            &harsh.encode_hex("FA").expect("failed to encode"),
            "error encoding `FA`"
        );
        assert_eq!(
            "MemE",
            &harsh.encode_hex("26dd").expect("failed to encode"),
            "error encoding `26dd`"
        );
        assert_eq!(
            "eBMrb",
            &harsh.encode_hex("FF1A").expect("failed to encode"),
            "error encoding `FF1A`"
        );
        assert_eq!(
            "D9NPE",
            &harsh.encode_hex("12abC").expect("failed to encode"),
            "error encoding `12abC`"
        );
        assert_eq!(
            "9OyNW",
            &harsh.encode_hex("185b0").expect("failed to encode"),
            "error encoding `185b0`"
        );
        assert_eq!(
            "MRWNE",
            &harsh.encode_hex("17b8d").expect("failed to encode"),
            "error encoding `17b8d`"
        );
        assert_eq!(
            "4o6Z7KqxE",
            &harsh.encode_hex("1d7f21dd38").expect("failed to encode"),
            "error encoding `1d7f21dd38`"
        );
        assert_eq!(
            "ooweQVNB",
            &harsh.encode_hex("20015111d").expect("failed to encode"),
            "error encoding `20015111d`"
        );
        assert_eq!(
            "kRNrpKlJ",
            &harsh.encode_hex("deadbeef").expect("failed to encode"),
            "error encoding `deadbeef`"
        );

        let harsh = HarshBuilder::new().init().unwrap();
        assert_eq!(
            "y42LW46J9luq3Xq9XMly",
            &harsh
                .encode_hex("507f1f77bcf86cd799439011",)
                .expect("failed to encode",),
            "error encoding `507f1f77bcf86cd799439011`"
        );
    }

    #[test]
    fn can_encode_hex_with_guards() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(10)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "GkRNrpKlJd",
            &harsh.encode_hex("deadbeef").expect("failed to encode")
        );
    }

    #[test]
    fn can_encode_hex_with_padding() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(12)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "RGkRNrpKlJde",
            &harsh.encode_hex("deadbeef").expect("failed to encode")
        );
    }

    #[test]
    fn can_decode_hex() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "fa",
            harsh.decode_hex("lzY").expect("failed to decode"),
            "error decoding `FA`"
        );
        assert_eq!(
            "26dd",
            harsh.decode_hex("MemE").expect("failed to decode"),
            "error decoding `26dd`"
        );
        assert_eq!(
            "ff1a",
            harsh.decode_hex("eBMrb").expect("failed to decode"),
            "error decoding `FF1A`"
        );
        assert_eq!(
            "12abc",
            harsh.decode_hex("D9NPE").expect("failed to decode"),
            "error decoding `12abC`"
        );
        assert_eq!(
            "185b0",
            harsh.decode_hex("9OyNW").expect("failed to decode"),
            "error decoding `185b0`"
        );
        assert_eq!(
            "17b8d",
            harsh.decode_hex("MRWNE").expect("failed to decode"),
            "error decoding `17b8d`"
        );
        assert_eq!(
            "1d7f21dd38",
            harsh.decode_hex("4o6Z7KqxE").expect("failed to decode"),
            "error decoding `1d7f21dd38`"
        );
        assert_eq!(
            "20015111d",
            harsh.decode_hex("ooweQVNB").expect("failed to decode"),
            "error decoding `20015111d`"
        );
        assert_eq!(
            "deadbeef",
            harsh.decode_hex("kRNrpKlJ").expect("failed to decode"),
            "error decoding `deadbeef`"
        );

        let harsh = HarshBuilder::new().init().unwrap();
        assert_eq!(
            "507f1f77bcf86cd799439011",
            harsh
                .decode_hex("y42LW46J9luq3Xq9XMly",)
                .expect("failed to decode",),
            "error decoding `y42LW46J9luq3Xq9XMly`"
        );
    }

    #[test]
    fn can_decode_hex_with_guards() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(10)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "deadbeef",
            harsh.decode_hex("GkRNrpKlJd").expect("failed to decode"),
            "failed to decode GkRNrpKlJd"
        );
    }

    #[test]
    fn can_decode_hex_with_padding() {
        let harsh = HarshBuilder::new()
            .salt("this is my salt")
            .length(12)
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "deadbeef",
            harsh.decode_hex("RGkRNrpKlJde").expect("failed to decode"),
            "failed to decode RGkRNrpKlJde"
        );
    }

    #[test]
    fn can_encode_with_custom_alphabet() {
        let harsh = HarshBuilder::new()
            .alphabet("abcdefghijklmnopqrstuvwxyz")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            "mdfphx",
            harsh.encode(&[1, 2, 3]).expect("failed to encode"),
            "failed to encode [1, 2, 3]"
        );
    }

    #[test]
    fn can_decode_with_invalid_alphabet() {
        let harsh = Harsh::default();
        assert_eq!(None, harsh.decode("this$ain't|a\number"));
    }

    #[test]
    fn can_decode_with_custom_alphabet() {
        let harsh = HarshBuilder::new()
            .alphabet("abcdefghijklmnopqrstuvwxyz")
            .init()
            .expect("failed to initialize harsh");

        assert_eq!(
            &[1, 2, 3],
            &harsh.decode("mdfphx").expect("failed to decode")[..],
            "failed to decode mdfphx"
        );
    }

    #[test]
    fn create_nhash() {
        let values = &[1, 2, 3];
        let nhash = super::create_nhash(values);
        assert_eq!(6, nhash);
    }

    #[test]
    fn hash() {
        let result = super::hash(22, b"abcdefghijklmnopqrstuvwxyz");
        assert_eq!("w", result);
    }

    #[test]
    fn alphabet_and_separator_generation() {
        use super::{DEFAULT_ALPHABET, DEFAULT_SEPARATORS};

        let (alphabet, separators) = super::alphabet_and_separators(
            &Some(DEFAULT_SEPARATORS.to_vec()),
            DEFAULT_ALPHABET,
            b"this is my salt",
        );

        assert_eq!(
            "AdG05N6y2rljDQak4xgzn8ZR1oKYLmJpEbVq3OBv9WwXPMe7",
            alphabet.iter().map(|&u| u as char).collect::<String>()
        );
        assert_eq!(
            "UHuhtcITCsFifS",
            separators.iter().map(|&u| u as char).collect::<String>()
        );
    }

    #[test]
    fn alphabet_and_separator_generation_with_few_separators() {
        use super::DEFAULT_ALPHABET;

        let separators = b"fu";
        let (alphabet, separators) = super::alphabet_and_separators(
            &Some(separators.to_vec()),
            DEFAULT_ALPHABET,
            b"this is my salt",
        );

        assert_eq!(
            "4RVQrYM87wKPNSyTBGU1E6FIC9ALtH0ZD2Wxz3vs5OXJ",
            alphabet.iter().map(|&u| u as char).collect::<String>()
        );
        assert_eq!(
            "ufabcdeghijklmnopq",
            separators.iter().map(|&u| u as char).collect::<String>()
        );
    }

    #[test]
    fn shuffle() {
        let salt = b"1234";
        let mut values = "asdfzxcvqwer".bytes().collect::<Vec<_>>();
        super::shuffle(&mut values, salt);

        assert_eq!("vdwqfrzcsxae", String::from_utf8_lossy(&values));
    }

    #[test]
    fn guard_characters_should_be_added_to_left_first() {
        let harsh = HarshBuilder::new().length(3).init().unwrap();
        let hashed_value = harsh.encode(&[1]).unwrap();

        assert_eq!(&hashed_value, "ejR");
        assert_eq!(
            Some(vec![1]),
            harsh.decode("ejR"),
            "should return None when decoding a valid id with a garbage ending",
        );
    }
}
