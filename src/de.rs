//! Deserialization.

use core::convert::TryInto;
use core::f32;
use core::marker::PhantomData;
use core::result;
use core::str;
use half::f16;
use serde::de;
#[cfg(feature = "std")]
use std::io;

use crate::error::{Error, ErrorCode, ExpectedSet, Result};
#[cfg(not(feature = "unsealed_read_write"))]
use crate::read::EitherLifetime;
#[cfg(feature = "unsealed_read_write")]
pub use crate::read::EitherLifetime;
#[cfg(feature = "std")]
pub use crate::read::IoRead;
use crate::read::Offset;
#[cfg(any(feature = "std", feature = "alloc"))]
pub use crate::read::SliceRead;
pub use crate::read::{MutSliceRead, Read, SliceReadFixed};
#[cfg(feature = "tags")]
use crate::tags::set_tag;
/// Decodes a value from CBOR data in a slice.
///
/// # Examples
///
/// Deserialize a `String`
///
/// ```
/// # use serde_cbor::de;
/// let v: Vec<u8> = vec![0x66, 0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72];
/// let value: String = de::from_slice(&v[..]).unwrap();
/// assert_eq!(value, "foobar");
/// ```
///
/// Deserialize a borrowed string with zero copies.
///
/// ```
/// # use serde_cbor::de;
/// let v: Vec<u8> = vec![0x66, 0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72];
/// let value: &str = de::from_slice(&v[..]).unwrap();
/// assert_eq!(value, "foobar");
/// ```
#[cfg(any(feature = "std", feature = "alloc"))]
pub fn from_slice<'a, T>(slice: &'a [u8]) -> Result<T>
where
    T: de::Deserialize<'a>,
{
    let mut deserializer = Deserializer::from_slice(slice);
    let value = de::Deserialize::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

// When the "std" feature is enabled there should be little to no need to ever use this function,
// as `from_slice` covers all use cases (at the expense of being less efficient).
/// Decode a value from CBOR data in a mutable slice.
///
/// This can be used in analogy to `from_slice`. Unlike `from_slice`, this will use the slice's
/// mutability to rearrange data in it in order to resolve indefinite byte or text strings without
/// resorting to allocations.
pub fn from_mut_slice<'a, T>(slice: &'a mut [u8]) -> Result<T>
where
    T: de::Deserialize<'a>,
{
    let mut deserializer = Deserializer::from_mut_slice(slice);
    let value = de::Deserialize::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

// When the "std" feature is enabled there should be little to no need to ever use this function,
// as `from_slice` covers all use cases and is much more reliable (at the expense of being less
// efficient).
/// Decode a value from CBOR data using a scratch buffer.
///
/// Users should generally prefer to use `from_slice` or `from_mut_slice` over this function,
/// as decoding may fail when the scratch buffer turns out to be too small.
///
/// A realistic use case for this method would be decoding in a `no_std` environment from an
/// immutable slice that is too large to copy.
pub fn from_slice_with_scratch<'a, 'b, T>(slice: &'a [u8], scratch: &'b mut [u8]) -> Result<T>
where
    T: de::Deserialize<'a>,
{
    let mut deserializer = Deserializer::from_slice_with_scratch(slice, scratch);
    let value = de::Deserialize::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// Decodes a value from CBOR data in a reader.
///
/// # Examples
///
/// Deserialize a `String`
///
/// ```
/// # use serde_cbor::de;
/// let v: Vec<u8> = vec![0x66, 0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72];
/// let value: String = de::from_reader(&v[..]).unwrap();
/// assert_eq!(value, "foobar");
/// ```
///
/// Note that `from_reader` cannot borrow data:
///
/// ```compile_fail
/// # use serde_cbor::de;
/// let v: Vec<u8> = vec![0x66, 0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72];
/// let value: &str = de::from_reader(&v[..]).unwrap();
/// assert_eq!(value, "foobar");
/// ```
#[cfg(feature = "std")]
pub fn from_reader<T, R>(reader: R) -> Result<T>
where
    T: de::DeserializeOwned,
    R: io::Read + Send,
{
    let mut deserializer = Deserializer::from_reader(reader);
    let value = de::Deserialize::deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

/// A Serde `Deserialize`r of CBOR data.
#[derive(Debug)]
pub struct Deserializer<R, O = DefaultDeserializerOptions> {
    read: R,
    remaining_depth: u8,
    options: O,
}

#[cfg(feature = "std")]
impl<R> Deserializer<IoRead<R>>
where
    R: io::Read,
{
    /// Constructs a `Deserializer` which reads from a `Read`er.
    pub fn from_reader(reader: R) -> Deserializer<IoRead<R>> {
        Deserializer::new(IoRead::new(reader))
    }
}

#[cfg(any(feature = "std", feature = "alloc"))]
impl<'a> Deserializer<SliceRead<'a>> {
    /// Constructs a `Deserializer` which reads from a slice.
    ///
    /// Borrowed strings and byte slices will be provided when possible.
    pub fn from_slice(bytes: &'a [u8]) -> Deserializer<SliceRead<'a>> {
        Deserializer::new(SliceRead::new(bytes))
    }
}

impl<'a> Deserializer<MutSliceRead<'a>> {
    /// Constructs a `Deserializer` which reads from a mutable slice that doubles as its own
    /// scratch buffer.
    ///
    /// Borrowed strings and byte slices will be provided even for indefinite strings.
    pub fn from_mut_slice(bytes: &'a mut [u8]) -> Deserializer<MutSliceRead<'a>> {
        Deserializer::new(MutSliceRead::new(bytes))
    }
}

impl<'a, 'b> Deserializer<SliceReadFixed<'a, 'b>> {
    #[doc(hidden)]
    pub fn from_slice_with_scratch(
        bytes: &'a [u8],
        scratch: &'b mut [u8],
    ) -> Deserializer<SliceReadFixed<'a, 'b>> {
        Deserializer::new(SliceReadFixed::new(bytes, scratch))
    }
}

/// Deserializer Options trait
#[allow(missing_docs)]
pub trait DeserializerOptions {
    #[inline]
    fn accept_named(&self) -> bool {
        true
    }

    #[inline]
    fn accept_packed(&self) -> bool {
        true
    }

    #[inline]
    fn accept_standard_enums(&self) -> bool {
        true
    }

    #[inline]
    fn accept_legacy_enums(&self) -> bool {
        false
    }

    #[inline]
    fn to_custom(&self) -> CustomDeserializerOptions {
        CustomDeserializerOptions {
            accept_named: self.accept_named(),
            accept_packed: self.accept_packed(),
            accept_standard_enums: self.accept_standard_enums(),
            accept_legacy_enums: self.accept_legacy_enums(),
        }
    }
}

/// Default Deserializer Options
#[derive(Debug)]
pub struct DefaultDeserializerOptions;

/// Custom Deserializer Options
#[derive(Debug)]
pub struct CustomDeserializerOptions {
    accept_named: bool,
    accept_packed: bool,
    accept_standard_enums: bool,
    accept_legacy_enums: bool,
}

impl Default for CustomDeserializerOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl DeserializerOptions for CustomDeserializerOptions {
    #[inline]
    fn accept_named(&self) -> bool {
        self.accept_named
    }
    #[inline]
    fn accept_packed(&self) -> bool {
        self.accept_packed
    }
    #[inline]
    fn accept_standard_enums(&self) -> bool {
        self.accept_standard_enums
    }
    #[inline]
    fn accept_legacy_enums(&self) -> bool {
        self.accept_legacy_enums
    }
}

impl CustomDeserializerOptions {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        DefaultDeserializerOptions.to_custom()
    }

    /// Accept named variants and fields.
    pub fn set_accept_named_format(mut self, new: bool) -> Self {
        self.accept_named = new;
        self
    }

    /// Accept numbered variants and fields.
    pub fn set_accept_packed_format(mut self, new: bool) -> Self {
        self.accept_packed = new;
        self
    }

    /// Accept the new enum format used by `serde_cbor` versions >= v0.10.
    pub fn set_accept_standard_enums(mut self, new: bool) -> Self {
        self.accept_standard_enums = new;
        self
    }

    /// Accept the old enum format used by `serde_cbor` versions <= v0.9.
    pub fn set_accept_legacy_enums(mut self, new: bool) -> Self {
        self.accept_legacy_enums = new;
        self
    }
}

impl DeserializerOptions for DefaultDeserializerOptions {}

impl<'de, R> Deserializer<R>
where
    R: Read<'de>,
{
    /// Constructs a `Deserializer` from one of the possible serde_cbor input sources.
    ///
    /// `from_slice` and `from_reader` should normally be used instead of this method.
    #[inline]
    pub fn new(read: R) -> Self {
        Deserializer::new_with_options(read, DefaultDeserializerOptions)
    }
}

impl<'de, R, O> Deserializer<R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    /// Constructs a `Deserializer` from one of the possible serde_cbor input sources.
    ///
    /// `from_slice` and `from_reader` should normally be used instead of this method.
    #[inline]
    pub fn new_with_options(read: R, options: O) -> Self {
        Deserializer {
            read,
            remaining_depth: 128,
            options,
        }
    }

    /// Don't accept named variants and fields.
    #[inline]
    pub fn disable_named_format(self) -> Deserializer<R, CustomDeserializerOptions> {
        Deserializer {
            read: self.read,
            remaining_depth: self.remaining_depth,
            options: self.options.to_custom().set_accept_named_format(false),
        }
    }

    /// Don't accept numbered variants and fields.
    #[inline]
    pub fn disable_packed_format(self) -> Deserializer<R, CustomDeserializerOptions> {
        Deserializer {
            read: self.read,
            remaining_depth: self.remaining_depth,
            options: self.options.to_custom().set_accept_packed_format(false),
        }
    }

    /// Don't accept the new enum format used by `serde_cbor` versions >= v0.10.
    #[inline]
    pub fn disable_standard_enums(self) -> Deserializer<R, CustomDeserializerOptions> {
        Deserializer {
            read: self.read,
            remaining_depth: self.remaining_depth,
            options: self.options.to_custom().set_accept_standard_enums(false),
        }
    }

    /// Don't accept the old enum format used by `serde_cbor` versions <= v0.9.
    #[inline]
    pub fn disable_legacy_enums(self) -> Deserializer<R, CustomDeserializerOptions> {
        Deserializer {
            read: self.read,
            remaining_depth: self.remaining_depth,
            options: self.options.to_custom().set_accept_legacy_enums(false),
        }
    }

    /// This method should be called after a value has been deserialized to ensure there is no
    /// trailing data in the input source.
    pub fn end(&mut self) -> Result<()> {
        match self.next()? {
            Some(_) => Err(self.error(ErrorCode::TrailingData)),
            None => Ok(()),
        }
    }

    /// Turn a CBOR deserializer into an iterator over values of type T.
    #[allow(clippy::should_implement_trait)] // Trait doesn't allow unconstrained T.
    pub fn into_iter<T>(self) -> StreamDeserializer<'de, R, T, O>
    where
        T: de::Deserialize<'de>,
    {
        StreamDeserializer {
            de: self,
            output: PhantomData,
            lifetime: PhantomData,
        }
    }

    #[inline]
    fn next(&mut self) -> Result<Option<u8>> {
        self.read.next()
    }

    #[inline]
    fn peek(&mut self) -> Result<Option<u8>> {
        self.read.peek()
    }

    #[inline]
    fn consume(&mut self) {
        self.read.discard();
    }

    #[cold]
    fn error(&self, reason: ErrorCode) -> Error {
        let offset = self.read.offset();
        Error::syntax(reason, offset)
    }

    #[inline]
    fn parse_uint(&mut self, magnitude: u8) -> Result<u64> {
        let mut buf = [0; 8];
        let bytes = 1 << (magnitude - 1);
        let buf_view = &mut buf[8 - bytes..];
        self.read.read_into(buf_view)?;
        Ok(u64::from_be_bytes(buf))
    }

    #[inline]
    fn parse_u8(&mut self) -> Result<u8> {
        match self.next()? {
            Some(byte) => Ok(byte),
            None => Err(self.error(ErrorCode::EofWhileParsingValue)),
        }
    }

    fn parse_bytes<V>(&mut self, len: Option<usize>, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let read = if let Some(len) = len {
            self.read.read(len)
        } else {
            self.read_indefinite_bytes()
        }?;
        match read {
            EitherLifetime::Long(buf) => visitor.visit_borrowed_bytes(buf),
            EitherLifetime::Short(buf) => visitor.visit_bytes(buf),
        }
    }

    #[cold]
    fn read_indefinite_bytes(&mut self) -> Result<EitherLifetime<'_, 'de>> {
        self.read.clear_buffer();
        loop {
            let byte = self.parse_u8()?;
            let len = match byte {
                0x40..=0x57 => byte as usize - 0x40,
                0x58..=0x5b => {
                    let len = self.parse_uint(byte - 0x57)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    len as usize
                }
                0xff => break,
                _ => return Err(self.error(ErrorCode::UnexpectedCode(ExpectedSet::STRING, byte))),
            };

            self.read.read_to_buffer(len)?;
        }

        Ok(self.read.take_buffer())
    }

    fn convert_str<'a>(buf: &'a [u8], offset: u64) -> Result<&'a str> {
        match str::from_utf8(buf) {
            Ok(s) => Ok(s),
            Err(_) => Err(Error::syntax(ErrorCode::InvalidUtf8, offset)),
        }
    }

    fn parse_str<V>(&mut self, len: Option<usize>, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let offset = self.read.offset();
        let read = if let Some(len) = len {
            self.read.read(len)
        } else {
            self.read_indefinite_str()
        }?;
        match read {
            EitherLifetime::Long(buf) => {
                let s = Self::convert_str(buf, offset)?;
                visitor.visit_borrowed_str(s)
            }
            EitherLifetime::Short(buf) => {
                let s = Self::convert_str(buf, offset)?;
                visitor.visit_str(s)
            }
        }
    }

    #[cold]
    fn read_indefinite_str(&mut self) -> Result<EitherLifetime<'_, 'de>> {
        self.read.clear_buffer();
        loop {
            let byte = self.parse_u8()?;
            let len = match byte {
                0x60..=0x77 => byte as usize - 0x60,
                0x78..=0x7b => {
                    let len = self.parse_uint(byte - 0x77)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    len as usize
                }
                0xff => break,
                _ => return Err(self.error(ErrorCode::UnexpectedCode(ExpectedSet::STRING, byte))),
            };

            self.read.read_to_buffer(len)?;
        }

        Ok(self.read.take_buffer())
    }

    #[cfg(feature = "tags")]
    fn handle_tagged_value<V>(&mut self, tag: u64, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.recursion_checked(|d| {
            set_tag(Some(tag));
            let r = visitor.visit_newtype_struct(d);
            set_tag(None);
            r
        })
    }

    #[cfg(not(feature = "tags"))]
    fn handle_tagged_value<V, Valid>(&mut self, _tag: u64, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
        Valid: ValidValues,
    {
        self.recursion_checked(|de| de.parse_value::<_, Valid>(visitor))
    }

    fn recursion_checked<F, T>(&mut self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Deserializer<R, O>) -> Result<T>,
    {
        self.remaining_depth -= 1;
        if self.remaining_depth == 0 {
            return Err(self.error(ErrorCode::RecursionLimitExceeded));
        }
        let r = f(self);
        self.remaining_depth += 1;
        r
    }

    fn parse_array<V>(&mut self, mut len: Option<usize>, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.recursion_checked(|de| {
            let value = visitor.visit_seq(SeqAccess {
                de,
                len: len.as_mut(),
            })?;

            match len {
                Some(0) => (),
                Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                None => match de.next()? {
                    Some(0xff) => (),
                    Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                    None => return Err(de.error(ErrorCode::EofWhileParsingArray)),
                },
            }
            Ok(value)
        })
    }

    fn parse_map<V>(&mut self, mut len: Option<usize>, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.recursion_checked(|de| {
            let value = visitor.visit_map(MapAccess {
                de,
                len: len.as_mut(),
            })?;

            match len {
                Some(0) => (),
                Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                None => match de.next()? {
                    Some(0xff) => (),
                    Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                    None => return Err(de.error(ErrorCode::EofWhileParsingMap)),
                },
            }
            Ok(value)
        })
    }

    fn parse_enum<V>(&mut self, mut len: Option<usize>, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.recursion_checked(|de| {
            let value = visitor.visit_enum(VariantAccess {
                seq: SeqAccess {
                    de,
                    len: len.as_mut(),
                },
            })?;

            match len {
                Some(0) => (),
                Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                None => match de.next()? {
                    Some(0xff) => (),
                    Some(_) => return Err(de.error(ErrorCode::TrailingData)),
                    None => return Err(de.error(ErrorCode::EofWhileParsingArray)),
                },
            }
            Ok(value)
        })
    }

    fn parse_enum_map<V>(&mut self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.recursion_checked(|de| {
            let mut len = 1;
            let value = visitor.visit_enum(VariantAccessMap {
                map: MapAccess {
                    de,
                    len: Some(&mut len),
                },
            })?;

            if len != 0 {
                Err(de.error(ErrorCode::TrailingData))
            } else {
                Ok(value)
            }
        })
    }

    fn parse_float(&mut self, magnitude: u8) -> Result<f64> {
        let mut buf = [0; 8];
        let bytes = 1 << (magnitude - 1);
        self.read.read_into(&mut buf[..bytes])?;
        Ok(match magnitude {
            2 => f16::from_be_bytes(buf[..2].try_into().unwrap()).to_f64(),
            3 => f32::from_be_bytes(buf[..4].try_into().unwrap()) as f64,
            4 => f64::from_be_bytes(buf[..8].try_into().unwrap()),
            _ => unreachable!(),
        })
    }

    // Don't warn about the `unreachable!` in case
    // exhaustive integer pattern matching is enabled.
    #[allow(unreachable_patterns)]
    fn parse_value<V, Valid>(&mut self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
        Valid: ValidValues,
    {
        let byte = self.parse_u8()?;
        match byte {
            // Major type 0: an unsigned integer
            0x00..=0x1b if Valid::INT_POS => {
                let value = if byte <= 0x17 {
                    byte as u64
                } else {
                    self.parse_uint(byte - 0x17)?
                };
                visitor.visit_u64(value)
            }

            // Major type 1: a negative integer
            0x20..=0x3b if Valid::INT_NEG => {
                let u_value = if byte <= 0x37 {
                    (byte - 0x20) as u64
                } else {
                    let u_value = self.parse_uint(byte - 0x37)?;
                    if u_value > i64::max_value() as u64 {
                        return visitor.visit_i128(-1 - i128::from(u_value));
                    }
                    u_value
                };
                visitor.visit_i64(-1 - u_value as i64)
            }

            // Major type 2: a byte string
            0x40..=0x5b | 0x5f if Valid::BYTES => {
                let len = if byte == 0x5f {
                    None
                } else if byte <= 0x57 {
                    Some(byte as usize - 0x40)
                } else {
                    let len = self.parse_uint(byte - 0x57)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    Some(len as usize)
                };
                self.parse_bytes(len, visitor)
            }

            // Major type 3: a text string
            0x60..=0x7b | 0x7f if Valid::STRING => {
                let len = if byte == 0x7f {
                    None
                } else if byte <= 0x77 {
                    Some(byte as usize - 0x60)
                } else {
                    let len = self.parse_uint(byte - 0x77)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    Some(len as usize)
                };
                self.parse_str(len, visitor)
            }

            // Major type 4: an array of data items
            0x80..=0x9b | 0x9f if Valid::ARRAY => {
                let len = if byte == 0x9f {
                    None
                } else if byte <= 0x97 {
                    Some(byte as usize - 0x80)
                } else {
                    let len = self.parse_uint(byte - 0x97)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    Some(len as usize)
                };
                self.parse_array(len, visitor)
            }

            // Major type 5: a map of pairs of data items
            0xa0..=0xbb | 0xbf if Valid::MAP => {
                let len = if byte == 0xbf {
                    None
                } else if byte <= 0xb7 {
                    Some(byte as usize - 0xa0)
                } else {
                    let len = self.parse_uint(byte - 0xb7)?;
                    if len > usize::max_value() as u64 {
                        return Err(self.error(ErrorCode::LengthOutOfRange));
                    }
                    Some(len as usize)
                };
                self.parse_map(len, visitor)
            }

            // Major type 6: optional semantic tagging of other major types
            0xc0..=0xdb => {
                let tag = if byte <= 0xd7 {
                    byte as u64 - 0xc0
                } else {
                    self.parse_uint(byte - 0xd7)?
                };
                self.handle_tagged_value::<_, Valid>(tag, visitor)
            }

            // Major type 7: floating-point numbers and other simple data types that need no content
            0xf4..=0xf5 if Valid::BOOL => visitor.visit_bool(byte == 0xf5),
            0xf6..=0xf7 if Valid::NULL => visitor.visit_unit(),
            // 0xf8 => Err(self.error(ErrorCode::UnassignedCode)),
            0xf9..=0xfb if Valid::FLOAT => {
                let value = self.parse_float(byte - 0xf9 + 2)?;
                visitor.visit_f64(value)
            }
            _ => Err(self.error(ErrorCode::UnexpectedCode(
                ExpectedSet::from_valid::<Valid>(),
                byte,
            ))),
        }
    }
}

impl<'de, 'a, R, O> de::Deserializer<'de> for &'a mut Deserializer<R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    type Error = Error;

    #[inline]
    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidAll>(visitor)
    }

    #[inline]
    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.peek()? {
            Some(0xf6) => {
                self.consume();
                visitor.visit_none()
            }
            _ => visitor.visit_some(self),
        }
    }

    #[inline]
    fn deserialize_newtype_struct<V>(self, _name: &str, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    // Unit variants are encoded as just the variant identifier.
    // Tuple variants are encoded as an array of the variant identifier followed by the fields.
    // Struct variants are encoded as an array of the variant identifier followed by the struct.
    #[inline]
    fn deserialize_enum<V>(
        self,
        _name: &str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.peek()? {
            Some(byte @ 0x80..=0x9b | byte @ 0x9f) => {
                if !self.options.accept_legacy_enums() {
                    return Err(self.error(ErrorCode::WrongEnumFormat));
                }
                self.consume();
                match byte {
                    0x80..=0x9b | 0x9f => {
                        let len = if byte == 0x9f {
                            None
                        } else if byte <= 0x97 {
                            Some(byte as usize - 0x80)
                        } else {
                            let len = self.parse_uint(byte - 0x97)?;
                            if len > usize::max_value() as u64 {
                                return Err(self.error(ErrorCode::LengthOutOfRange));
                            }
                            Some(len as usize)
                        };
                        self.parse_enum(len, visitor)
                    }
                    _ => unreachable!(),
                }
            }
            Some(0xa1) => {
                if !self.options.accept_standard_enums() {
                    return Err(self.error(ErrorCode::WrongEnumFormat));
                }
                self.consume();
                self.parse_enum_map(visitor)
            }
            None => Err(self.error(ErrorCode::EofWhileParsingValue)),
            _ => {
                if !self.options.accept_standard_enums() && !self.options.accept_legacy_enums() {
                    return Err(self.error(ErrorCode::WrongEnumFormat));
                }
                visitor.visit_enum(UnitVariantAccess { de: self })
            }
        }
    }

    #[inline]
    fn is_human_readable(&self) -> bool {
        false
    }

    fn deserialize_bool<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForBool>(visitor)
    }

    fn deserialize_i8<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i16<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i32<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_i64<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForSInt>(visitor)
    }

    fn deserialize_u8<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u16<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u32<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }

    fn deserialize_u64<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForUInt>(visitor)
    }

    fn deserialize_str<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForString>(visitor)
    }

    fn deserialize_string<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_map<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForMap>(visitor)
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForStringAndUInt>(visitor)
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_bytes<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForBytes>(visitor)
    }

    fn deserialize_seq<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForSeq>(visitor)
    }

    fn deserialize_char<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_f32<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_f64(visitor)
    }

    fn deserialize_f64<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForFloat>(visitor)
    }

    fn deserialize_unit<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidForUnit>(visitor)
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_tuple(len, visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.parse_value::<_, ValidAll>(visitor)
    }

    fn deserialize_i128<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_i64(visitor)
    }

    fn deserialize_u128<V>(self, visitor: V) -> result::Result<V::Value, Self::Error>
    where
        V: de::Visitor<'de>,
    {
        self.deserialize_u64(visitor)
    }
}

impl<R, O> Deserializer<R, O>
where
    R: Offset,
    O: DeserializerOptions,
{
    /// Return the current offset in the reader
    #[inline]
    pub fn byte_offset(&self) -> usize {
        self.read.byte_offset()
    }
}

trait MakeError {
    fn error(&self, code: ErrorCode) -> Error;
}

struct SeqAccess<'a, R, O> {
    de: &'a mut Deserializer<R, O>,
    len: Option<&'a mut usize>,
}

impl<'de, 'a, R, O> de::SeqAccess<'de> for SeqAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        if let Some(len) = &mut self.len {
            if **len == 0 {
                return Ok(None);
            }
            **len -= 1;
        } else {
            match self.de.peek()? {
                Some(0xff) => return Ok(None),
                Some(_) => (),
                None => return Err(self.de.error(ErrorCode::EofWhileParsingArray)),
            }
        }

        let value = seed.deserialize(&mut *self.de)?;
        Ok(Some(value))
    }

    fn size_hint(&self) -> Option<usize> {
        self.len.as_ref().map(|l| **l)
    }
}

impl<'de, 'a, R, O> MakeError for SeqAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    fn error(&self, code: ErrorCode) -> Error {
        self.de.error(code)
    }
}

struct MapAccess<'a, R, O> {
    de: &'a mut Deserializer<R, O>,
    len: Option<&'a mut usize>,
}

impl<'de, 'a, R, O> de::MapAccess<'de> for MapAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: de::DeserializeSeed<'de>,
    {
        if let Some(len) = &mut self.len {
            if **len == 0 {
                return Ok(None);
            }
            **len -= 1;
        } else {
            match self.de.peek()? {
                Some(0xff) => return Ok(None),
                Some(_) => (),
                None => return Err(self.de.error(ErrorCode::EofWhileParsingMap)),
            }
        }

        // TODO: the accept_packed check is broken here. If `accept packed` is `false`
        // the map deserializer will refuse integer keys (which are valid in cbor),
        // erroring with WrongStructFormat.
        if !self.de.options.accept_named() || !self.de.options.accept_packed() {
            match self.de.peek()? {
                Some(_byte @ 0x00..=0x1b) if !self.de.options.accept_packed() => {
                    return Err(self.de.error(ErrorCode::WrongStructFormat));
                }
                Some(_byte @ 0x60..=0x7f) if !self.de.options.accept_named() => {
                    return Err(self.de.error(ErrorCode::WrongStructFormat));
                }
                _ => {}
            };
        }

        let value = seed.deserialize(&mut *self.de)?;
        Ok(Some(value))
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: de::DeserializeSeed<'de>,
    {
        seed.deserialize(&mut *self.de)
    }

    fn size_hint(&self) -> Option<usize> {
        self.len.as_ref().map(|l| **l)
    }
}

impl<'de, 'a, R, O> MakeError for MapAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    fn error(&self, code: ErrorCode) -> Error {
        self.de.error(code)
    }
}

struct UnitVariantAccess<'a, R, O> {
    de: &'a mut Deserializer<R, O>,
}

impl<'de, 'a, R, O> de::EnumAccess<'de> for UnitVariantAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    type Error = Error;
    type Variant = UnitVariantAccess<'a, R, O>;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, UnitVariantAccess<'a, R, O>)>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = seed.deserialize(&mut *self.de)?;
        Ok((variant, self))
    }
}

impl<'de, 'a, R, O> de::VariantAccess<'de> for UnitVariantAccess<'a, R, O>
where
    R: Read<'de>,
    O: DeserializerOptions,
{
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        Ok(())
    }

    fn newtype_variant_seed<T>(self, _seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"newtype variant",
        ))
    }

    fn tuple_variant<V>(self, _len: usize, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"tuple variant",
        ))
    }

    fn struct_variant<V>(self, _fields: &'static [&'static str], _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        Err(de::Error::invalid_type(
            de::Unexpected::UnitVariant,
            &"struct variant",
        ))
    }
}

struct VariantAccess<T> {
    seq: T,
}

impl<'de, T> de::EnumAccess<'de> for VariantAccess<T>
where
    T: de::SeqAccess<'de, Error = Error> + MakeError,
{
    type Error = Error;
    type Variant = VariantAccess<T>;

    fn variant_seed<V>(mut self, seed: V) -> Result<(V::Value, VariantAccess<T>)>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = match self.seq.next_element_seed(seed) {
            Ok(Some(variant)) => variant,
            Ok(None) => return Err(self.seq.error(ErrorCode::ArrayTooShort)),
            Err(e) => return Err(e),
        };
        Ok((variant, self))
    }
}

impl<'de, T> de::VariantAccess<'de> for VariantAccess<T>
where
    T: de::SeqAccess<'de, Error = Error> + MakeError,
{
    type Error = Error;

    fn unit_variant(mut self) -> Result<()> {
        match self.seq.next_element() {
            Ok(Some(())) => Ok(()),
            Ok(None) => Err(self.seq.error(ErrorCode::ArrayTooLong)),
            Err(e) => Err(e),
        }
    }

    fn newtype_variant_seed<S>(mut self, seed: S) -> Result<S::Value>
    where
        S: de::DeserializeSeed<'de>,
    {
        match self.seq.next_element_seed(seed) {
            Ok(Some(variant)) => Ok(variant),
            Ok(None) => Err(self.seq.error(ErrorCode::ArrayTooShort)),
            Err(e) => Err(e),
        }
    }

    fn tuple_variant<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        visitor.visit_seq(self.seq)
    }

    fn struct_variant<V>(mut self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let seed = StructVariantSeed { visitor };
        match self.seq.next_element_seed(seed) {
            Ok(Some(variant)) => Ok(variant),
            Ok(None) => Err(self.seq.error(ErrorCode::ArrayTooShort)),
            Err(e) => Err(e),
        }
    }
}

struct StructVariantSeed<V> {
    visitor: V,
}

impl<'de, V> de::DeserializeSeed<'de> for StructVariantSeed<V>
where
    V: de::Visitor<'de>,
{
    type Value = V::Value;

    fn deserialize<D>(self, de: D) -> result::Result<V::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        de.deserialize_any(self.visitor)
    }
}

/// Iterator that deserializes a stream into multiple CBOR values.
///
/// A stream deserializer can be created from any CBOR deserializer using the
/// `Deserializer::into_iter` method.
///
/// ```
/// # extern crate serde_cbor;
/// use serde_cbor::de::Deserializer;
/// use serde_cbor::value::Value;
///
/// # fn main() {
/// let data: Vec<u8> = vec![
///     0x01, 0x66, 0x66, 0x6f, 0x6f, 0x62, 0x61, 0x72,
/// ];
/// let mut it = Deserializer::from_slice(&data[..]).into_iter::<Value>();
/// assert_eq!(
///     Value::Integer(1),
///     it.next().unwrap().unwrap()
/// );
/// assert_eq!(
///     Value::Text("foobar".to_string()),
///     it.next().unwrap().unwrap()
/// );
/// # }
/// ```
#[derive(Debug)]
pub struct StreamDeserializer<'de, R, T, O = DefaultDeserializerOptions> {
    de: Deserializer<R, O>,
    output: PhantomData<T>,
    lifetime: PhantomData<&'de ()>,
}

impl<'de, R, T> StreamDeserializer<'de, R, T>
where
    R: Read<'de>,
    T: de::Deserialize<'de>,
{
    /// Create a new CBOR stream deserializer from one of the possible
    /// serde_cbor input sources.
    ///
    /// Typically it is more convenient to use one of these methods instead:
    ///
    /// * `Deserializer::from_slice(...).into_iter()`
    /// * `Deserializer::from_reader(...).into_iter()`
    pub fn new(read: R) -> StreamDeserializer<'de, R, T> {
        StreamDeserializer {
            de: Deserializer::new(read),
            output: PhantomData,
            lifetime: PhantomData,
        }
    }
}

impl<'de, R, T, O> StreamDeserializer<'de, R, T, O>
where
    R: Offset,
    T: de::Deserialize<'de>,
    O: DeserializerOptions,
{
    /// Return the current offset in the reader
    #[inline]
    pub fn byte_offset(&self) -> usize {
        self.de.byte_offset()
    }
}

impl<'de, R, T, O> Iterator for StreamDeserializer<'de, R, T, O>
where
    R: Read<'de>,
    T: de::Deserialize<'de>,
    O: DeserializerOptions,
{
    type Item = Result<T>;

    fn next(&mut self) -> Option<Result<T>> {
        match self.de.peek() {
            Ok(Some(_)) => Some(T::deserialize(&mut self.de)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

struct VariantAccessMap<T> {
    map: T,
}

impl<'de, T> de::EnumAccess<'de> for VariantAccessMap<T>
where
    T: de::MapAccess<'de, Error = Error> + MakeError,
{
    type Error = Error;
    type Variant = VariantAccessMap<T>;

    fn variant_seed<V>(mut self, seed: V) -> Result<(V::Value, VariantAccessMap<T>)>
    where
        V: de::DeserializeSeed<'de>,
    {
        let variant = match self.map.next_key_seed(seed) {
            Ok(Some(variant)) => variant,
            Ok(None) => return Err(self.map.error(ErrorCode::ArrayTooShort)),
            Err(e) => return Err(e),
        };
        Ok((variant, self))
    }
}

impl<'de, T> de::VariantAccess<'de> for VariantAccessMap<T>
where
    T: de::MapAccess<'de, Error = Error> + MakeError,
{
    type Error = Error;

    fn unit_variant(mut self) -> Result<()> {
        match self.map.next_value() {
            Ok(()) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn newtype_variant_seed<S>(mut self, seed: S) -> Result<S::Value>
    where
        S: de::DeserializeSeed<'de>,
    {
        self.map.next_value_seed(seed)
    }

    fn tuple_variant<V>(mut self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let seed = StructVariantSeed { visitor };
        self.map.next_value_seed(seed)
    }

    fn struct_variant<V>(mut self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        let seed = StructVariantSeed { visitor };
        self.map.next_value_seed(seed)
    }
}

/// Customizes what `parse_value` will accept and generate code for
pub(crate) trait ValidValues {
    const STRING: bool = false;
    const BYTES: bool = false;
    const INT_POS: bool = false;
    const INT_NEG: bool = false;
    const FLOAT: bool = false;
    const ARRAY: bool = false;
    const MAP: bool = false;
    const BOOL: bool = false;
    const NULL: bool = false;
}
struct ValidAll;
impl ValidValues for ValidAll {
    const STRING: bool = true;
    const BYTES: bool = true;
    const INT_POS: bool = true;
    const INT_NEG: bool = true;
    const FLOAT: bool = true;
    const ARRAY: bool = true;
    const MAP: bool = true;
    const BOOL: bool = true;
    const NULL: bool = true;
}
struct ValidForString;
impl ValidValues for ValidForString {
    const STRING: bool = true;
    const BYTES: bool = true;
}
struct ValidForStringAndUInt;
impl ValidValues for ValidForStringAndUInt {
    const STRING: bool = true;
    const INT_POS: bool = true;
}
struct ValidForBytes;
impl ValidValues for ValidForBytes {
    const ARRAY: bool = true;
    const STRING: bool = true;
    const BYTES: bool = true;
}
struct ValidForSInt;
impl ValidValues for ValidForSInt {
    const INT_POS: bool = true;
    const INT_NEG: bool = true;
}
struct ValidForUInt;
impl ValidValues for ValidForUInt {
    const INT_POS: bool = true;
}
struct ValidForFloat;
impl ValidValues for ValidForFloat {
    const INT_POS: bool = true;
    const INT_NEG: bool = true;
    const FLOAT: bool = true;
}
struct ValidForSeq;
impl ValidValues for ValidForSeq {
    const ARRAY: bool = true;
}
struct ValidForMap;
impl ValidValues for ValidForMap {
    const MAP: bool = true;
}
struct ValidForBool;
impl ValidValues for ValidForBool {
    const BOOL: bool = true;
}
struct ValidForUnit;
impl ValidValues for ValidForUnit {
    const NULL: bool = true;
}
