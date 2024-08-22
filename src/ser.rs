//! Serialize a Rust data structure to CBOR data.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[cfg(feature = "std")]
pub use crate::write::IoWrite;
pub use crate::write::{SliceWrite, Write};

use crate::error::{Error, Result};
use half::f16;
use serde::ser::{self, Serialize};
#[cfg(feature = "std")]
use std::io;

use crate::tags::{get_tag, CBOR_NEWTYPE_NAME};

/// Serializes a value to a vector.
#[cfg(any(feature = "std", feature = "alloc"))]
pub fn to_vec<T>(value: &T) -> Result<Vec<u8>>
where
    T: ser::Serialize,
{
    let mut vec = Vec::new();
    value.serialize(&mut Serializer::new(&mut vec))?;
    Ok(vec)
}

/// Serializes a value to a vector in packed format.
#[cfg(feature = "std")]
pub fn to_vec_packed<T>(value: &T) -> Result<Vec<u8>>
where
    T: ser::Serialize,
{
    let options = CustomSerializerOptions {
        packed: true,
        emum_as_map: true,
    };
    let mut vec = Vec::new();
    let mut serializer = Serializer::new_with_options(&mut vec, options);
    value.serialize(&mut serializer)?;
    Ok(vec)
}

/// Serializes a value to a writer.
#[cfg(feature = "std")]
pub fn to_writer<W, T>(writer: W, value: &T) -> Result<()>
where
    W: io::Write,
    T: ser::Serialize,
{
    value.serialize(&mut Serializer::new(&mut IoWrite::new(writer)))
}

/// Serializer options
pub trait SerializerOptions {
    /// Choose concise/packed format for serializer.
    ///
    /// In the packed format enum variant names and field names
    /// are replaced with numeric indizes to conserve space.
    #[inline]
    fn packed(&self) -> bool {
        false
    }

    /// Enable old enum format used by `serde_cbor` versions <= v0.9.
    ///
    /// The `legacy_enums` option determines how enums are encoded.
    ///
    /// This makes no difference when encoding and decoding enums using
    /// this crate, but it shows up when decoding to a `Value` or decoding
    /// in other languages.
    ///
    /// # Examples
    ///
    /// Given the following enum
    ///
    /// ```rust
    /// enum Enum {
    ///     Unit,
    ///     NewType(i32),
    ///     Tuple(String, bool),
    ///     Struct{ x: i32, y: i32 },
    /// }
    /// ```
    /// we will give the `Value` with the same encoding for each case using
    /// JSON notation.
    ///
    /// ## Default encodings
    ///
    /// * `Enum::Unit` encodes as `"Unit"`
    /// * `Enum::NewType(10)` encodes as `{"NewType": 10}`
    /// * `Enum::Tuple("x", true)` encodes as `{"Tuple": ["x", true]}`
    ///
    /// ## Legacy encodings
    ///
    /// * `Enum::Unit` encodes as `"Unit"`
    /// * `Enum::NewType(10)` encodes as `["NewType", 10]`
    /// * `Enum::Tuple("x", true)` encodes as `["Tuple", "x", true]`
    /// * `Enum::Struct{ x: 5, y: -5 }` encodes as `["Struct", {"x": 5, "y": -5}]`
    #[inline]
    fn enum_as_map(&self) -> bool {
        true
    }

    #[allow(missing_docs)]
    #[inline]
    fn to_custom(&self) -> CustomSerializerOptions {
        CustomSerializerOptions {
            emum_as_map: self.enum_as_map(),
            packed: self.packed(),
        }
    }
}

/// Default serializer options
pub struct DefaultSerializerOptions;

impl SerializerOptions for DefaultSerializerOptions {}

/// Custom serializer options
pub struct CustomSerializerOptions {
    packed: bool,
    emum_as_map: bool,
}

#[allow(missing_docs)]
impl CustomSerializerOptions {
    #[inline]
    pub fn new() -> Self {
        DefaultSerializerOptions.to_custom()
    }

    #[inline]
    pub fn set_packed(mut self, new: bool) -> Self {
        self.packed = new;
        self
    }

    #[inline]
    pub fn set_emum_as_map(mut self, new: bool) -> Self {
        self.emum_as_map = new;
        self
    }
}

impl SerializerOptions for CustomSerializerOptions {
    #[inline]
    fn packed(&self) -> bool {
        self.packed
    }
    #[inline]
    fn enum_as_map(&self) -> bool {
        self.emum_as_map
    }
}

/// A structure for serializing Rust values to CBOR.
#[derive(Debug)]
pub struct Serializer<W, O = DefaultSerializerOptions> {
    writer: W,
    options: O,
}

impl<W> Serializer<W>
where
    W: Write,
{
    /// Creates a new CBOR serializer.
    ///
    /// `to_vec` and `to_writer` should normally be used instead of this method.
    #[inline]
    pub fn new(writer: W) -> Self {
        Serializer::new_with_options(writer, DefaultSerializerOptions)
    }
}

impl<W, O> Serializer<W, O>
where
    W: Write,
    O: SerializerOptions,
{
    /// Creates a new CBOR serializer.
    ///
    /// `to_vec` and `to_writer` should normally be used instead of this method.
    #[inline]
    pub fn new_with_options(writer: W, options: O) -> Self {
        Serializer { writer, options }
    }

    /// Choose concise/packed format for serializer.
    ///
    /// In the packed format enum variant names and field names
    /// are replaced with numeric indizes to conserve space.
    #[inline]
    pub fn packed_format(self) -> Serializer<W, CustomSerializerOptions> {
        Serializer {
            writer: self.writer,
            options: self.options.to_custom().set_packed(true),
        }
    }

    /// Enable old enum format used by `serde_cbor` versions <= v0.9.
    ///
    /// The `legacy_enums` option determines how enums are encoded.
    ///
    /// This makes no difference when encoding and decoding enums using
    /// this crate, but it shows up when decoding to a `Value` or decoding
    /// in other languages.
    ///
    /// # Examples
    ///
    /// Given the following enum
    ///
    /// ```rust
    /// enum Enum {
    ///     Unit,
    ///     NewType(i32),
    ///     Tuple(String, bool),
    ///     Struct{ x: i32, y: i32 },
    /// }
    /// ```
    /// we will give the `Value` with the same encoding for each case using
    /// JSON notation.
    ///
    /// ## Default encodings
    ///
    /// * `Enum::Unit` encodes as `"Unit"`
    /// * `Enum::NewType(10)` encodes as `{"NewType": 10}`
    /// * `Enum::Tuple("x", true)` encodes as `{"Tuple": ["x", true]}`
    ///
    /// ## Legacy encodings
    ///
    /// * `Enum::Unit` encodes as `"Unit"`
    /// * `Enum::NewType(10)` encodes as `["NewType", 10]`
    /// * `Enum::Tuple("x", true)` encodes as `["Tuple", "x", true]`
    /// * `Enum::Struct{ x: 5, y: -5 }` encodes as `["Struct", {"x": 5, "y": -5}]`
    #[inline]
    pub fn legacy_enums(self) -> Serializer<W, CustomSerializerOptions> {
        Serializer {
            writer: self.writer,
            options: self.options.to_custom().set_emum_as_map(false),
        }
    }

    /// Writes a CBOR self-describe tag to the stream.
    ///
    /// Tagging allows a decoder to distinguish different file formats based on their content
    /// without further information.
    #[inline]
    pub fn self_describe(&mut self) -> Result<()> {
        let mut buf = [6 << 5 | 25, 0, 0];
        (&mut buf[1..]).copy_from_slice(&55799u16.to_be_bytes());
        self.writer.write_all(&buf).map_err(|e| e.into())
    }

    /// Unwrap the `Writer` from the `Serializer`.
    #[inline]
    pub fn into_inner(self) -> W {
        self.writer
    }

    #[inline]
    fn write_u64(&mut self, major: u8, value: u64) -> Result<()> {
        let mut buf = [major << 5, 0, 0, 0, 0, 0, 0, 0, 0];
        let buf_view = if value <= 0x17 {
            buf[0] |= value as u8;
            &buf[..1]
        } else {
            if value <= u8::MAX as u64 {
                buf[0] |= 24;
                buf[1] = value as u8;
                &buf[..2]
            } else if value <= u16::MAX as u64 {
                buf[0] |= 25;
                (&mut buf[1..3]).copy_from_slice(&(value as u16).to_be_bytes());
                &buf[..3]
            } else if value <= u32::MAX as u64 {
                buf[0] |= 26;
                (&mut buf[1..5]).copy_from_slice(&(value as u32).to_be_bytes());
                &buf[..5]
            } else {
                buf[0] |= 27;
                (&mut buf[1..9]).copy_from_slice(&value.to_be_bytes());
                &buf[..9]
            }
        };
        self.writer.write_all(buf_view).map_err(|e| e.into())
    }

    #[inline]
    fn serialize_collection<'a>(
        &'a mut self,
        major: u8,
        len: Option<usize>,
    ) -> Result<CollectionSerializer<'a, W, O>> {
        let needs_eof = match len {
            Some(len) => {
                self.write_u64(major, len as u64)?;
                false
            }
            None => {
                self.writer
                    .write_all(&[major << 5 | 31])
                    .map_err(|e| e.into())?;
                true
            }
        };

        Ok(CollectionSerializer {
            ser: self,
            needs_eof,
        })
    }
}

impl<'a, W, O> ser::Serializer for &'a mut Serializer<W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    type SerializeSeq = CollectionSerializer<'a, W, O>;
    type SerializeTuple = &'a mut Serializer<W, O>;
    type SerializeTupleStruct = &'a mut Serializer<W, O>;
    type SerializeTupleVariant = &'a mut Serializer<W, O>;
    type SerializeMap = CollectionSerializer<'a, W, O>;
    type SerializeStruct = StructSerializer<'a, W, O>;
    type SerializeStructVariant = StructSerializer<'a, W, O>;

    #[inline]
    fn serialize_bool(self, value: bool) -> Result<()> {
        let value = if value { 0xf5 } else { 0xf4 };
        self.writer.write_all(&[value]).map_err(|e| e.into())
    }

    #[inline]
    fn serialize_i8(self, value: i8) -> Result<()> {
        self.serialize_i64(value as i64)
    }

    #[inline]
    fn serialize_i16(self, value: i16) -> Result<()> {
        self.serialize_i64(value as i64)
    }

    #[inline]
    fn serialize_i32(self, value: i32) -> Result<()> {
        self.serialize_i64(value as i64)
    }

    #[inline]
    fn serialize_i64(self, value: i64) -> Result<()> {
        let (major, u64_value) = if value < 0 {
            (1, -(value + 1) as u64)
        } else {
            (0, value as u64)
        };
        self.write_u64(major, u64_value)
    }

    #[inline]
    fn serialize_i128(self, value: i128) -> Result<()> {
        if value < 0 {
            if -(value + 1) > i128::from(u64::max_value()) {
                return Err(Error::message("The number can't be stored in CBOR"));
            }
            self.write_u64(1, -(value + 1) as u64)
        } else {
            if value > i128::from(u64::max_value()) {
                return Err(Error::message("The number can't be stored in CBOR"));
            }
            self.write_u64(0, value as u64)
        }
    }

    #[inline]
    fn serialize_u8(self, value: u8) -> Result<()> {
        self.write_u64(0, value as u64)
    }

    #[inline]
    fn serialize_u16(self, value: u16) -> Result<()> {
        self.write_u64(0, value as u64)
    }

    #[inline]
    fn serialize_u32(self, value: u32) -> Result<()> {
        self.write_u64(0, value as u64)
    }

    #[inline]
    fn serialize_u64(self, value: u64) -> Result<()> {
        self.write_u64(0, value)
    }

    #[inline]
    fn serialize_u128(self, value: u128) -> Result<()> {
        if value > u128::from(u64::max_value()) {
            return Err(Error::message("The number can't be stored in CBOR"));
        }
        self.write_u64(0, value as u64)
    }

    #[inline]
    #[allow(clippy::float_cmp)]
    fn serialize_f32(self, value: f32) -> Result<()> {
        if value.is_infinite() {
            if value.is_sign_positive() {
                self.writer.write_all(&[0xf9, 0x7c, 0x00])
            } else {
                self.writer.write_all(&[0xf9, 0xfc, 0x00])
            }
        } else if value.is_nan() {
            self.writer.write_all(&[0xf9, 0x7e, 0x00])
        } else if f32::from(f16::from_f32(value)) == value {
            let mut buf = [0xf9, 0, 0];
            (&mut buf[1..]).copy_from_slice(&f16::from_f32(value).to_bits().to_be_bytes());
            self.writer.write_all(&buf)
        } else {
            let mut buf = [0xfa, 0, 0, 0, 0];
            (&mut buf[1..]).copy_from_slice(&value.to_bits().to_be_bytes());
            self.writer.write_all(&buf)
        }
        .map_err(|e| e.into())
    }

    #[inline]
    #[allow(clippy::float_cmp)]
    fn serialize_f64(self, value: f64) -> Result<()> {
        if !value.is_finite() || f64::from(value as f32) == value {
            self.serialize_f32(value as f32)
        } else {
            let mut buf = [0xfb, 0, 0, 0, 0, 0, 0, 0, 0];
            (&mut buf[1..]).copy_from_slice(&value.to_bits().to_be_bytes());
            self.writer.write_all(&buf).map_err(|e| e.into())
        }
    }

    #[inline]
    fn serialize_char(self, value: char) -> Result<()> {
        // A char encoded as UTF-8 takes 4 bytes at most.
        let mut buf = [0; 4];
        self.serialize_str(value.encode_utf8(&mut buf))
    }

    #[inline]
    fn serialize_str(self, value: &str) -> Result<()> {
        self.write_u64(3, value.len() as u64)?;
        self.writer
            .write_all(value.as_bytes())
            .map_err(|e| e.into())
    }

    #[inline]
    fn serialize_bytes(self, value: &[u8]) -> Result<()> {
        self.write_u64(2, value.len() as u64)?;
        self.writer.write_all(value).map_err(|e| e.into())
    }

    #[inline]
    fn serialize_unit(self) -> Result<()> {
        self.serialize_none()
    }

    #[inline]
    fn serialize_some<T>(self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(self)
    }

    #[inline]
    fn serialize_none(self) -> Result<()> {
        self.writer.write_all(&[0xf6]).map_err(|e| e.into())
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
        self.serialize_unit()
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        variant: &'static str,
    ) -> Result<()> {
        if self.options.packed() {
            self.serialize_u32(variant_index)
        } else {
            self.serialize_str(variant)
        }
    }

    #[inline]
    fn serialize_newtype_struct<T>(self, name: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        if name == CBOR_NEWTYPE_NAME {
            for tag in get_tag().into_iter() {
                self.write_u64(6, tag)?;
            }
        }
        value.serialize(self)
    }

    #[inline]
    fn serialize_newtype_variant<T>(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        if self.options.enum_as_map() {
            self.write_u64(5, 1u64)?;
            variant.serialize(&mut *self)?;
        } else {
            self.writer.write_all(&[4 << 5 | 2]).map_err(|e| e.into())?;
            self.serialize_unit_variant(name, variant_index, variant)?;
        }
        value.serialize(self)
    }

    #[inline]
    fn serialize_seq(self, len: Option<usize>) -> Result<CollectionSerializer<'a, W, O>> {
        self.serialize_collection(4, len)
    }

    #[inline]
    fn serialize_tuple(self, len: usize) -> Result<&'a mut Serializer<W, O>> {
        self.write_u64(4, len as u64)?;
        Ok(self)
    }

    #[inline]
    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<&'a mut Serializer<W, O>> {
        self.serialize_tuple(len)
    }

    #[inline]
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<&'a mut Serializer<W, O>> {
        if self.options.enum_as_map() {
            self.write_u64(5, 1u64)?;
            variant.serialize(&mut *self)?;
            self.serialize_tuple(len)
        } else {
            self.write_u64(4, (len + 1) as u64)?;
            self.serialize_unit_variant(name, variant_index, variant)?;
            Ok(self)
        }
    }

    #[inline]
    fn serialize_map(self, len: Option<usize>) -> Result<CollectionSerializer<'a, W, O>> {
        self.serialize_collection(5, len)
    }

    #[cfg(not(feature = "std"))]
    fn collect_str<T: ?Sized>(self, value: &T) -> Result<()>
    where
        T: core::fmt::Display,
    {
        use crate::write::FmtWrite;
        use core::fmt::Write;

        let mut w = FmtWrite::new(&mut self.writer);
        write!(w, "{}", value)?;
        Ok(())
    }

    #[inline]
    fn serialize_struct(
        self,
        _name: &'static str,
        len: usize,
    ) -> Result<StructSerializer<'a, W, O>> {
        self.write_u64(5, len as u64)?;
        Ok(StructSerializer { ser: self, idx: 0 })
    }

    #[inline]
    fn serialize_struct_variant(
        self,
        name: &'static str,
        variant_index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<StructSerializer<'a, W, O>> {
        if self.options.enum_as_map() {
            self.write_u64(5, 1u64)?;
        } else {
            self.writer.write_all(&[4 << 5 | 2]).map_err(|e| e.into())?;
        }
        self.serialize_unit_variant(name, variant_index, variant)?;
        self.serialize_struct(name, len)
    }

    #[inline]
    fn is_human_readable(&self) -> bool {
        false
    }
}

impl<'a, W, O> ser::SerializeTuple for &'a mut Serializer<W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W, O> ser::SerializeTupleStruct for &'a mut Serializer<W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W, O> ser::SerializeTupleVariant for &'a mut Serializer<W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

#[doc(hidden)]
pub struct StructSerializer<'a, W, O> {
    ser: &'a mut Serializer<W, O>,
    idx: u32,
}

impl<'a, W, O> StructSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    #[inline]
    fn serialize_field_inner<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        if self.ser.options.packed() {
            self.idx.serialize(&mut *self.ser)?;
        } else {
            key.serialize(&mut *self.ser)?;
        }
        value.serialize(&mut *self.ser)?;
        self.idx += 1;
        Ok(())
    }

    #[inline]
    fn skip_field_inner(&mut self, _: &'static str) -> Result<()> {
        self.idx += 1;
        Ok(())
    }

    #[inline]
    fn end_inner(self) -> Result<()> {
        Ok(())
    }
}

impl<'a, W, O> ser::SerializeStruct for StructSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        self.serialize_field_inner(key, value)
    }

    #[inline]
    fn skip_field(&mut self, key: &'static str) -> Result<()> {
        self.skip_field_inner(key)
    }

    #[inline]
    fn end(self) -> Result<()> {
        self.end_inner()
    }
}

impl<'a, W, O> ser::SerializeStructVariant for StructSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        self.serialize_field_inner(key, value)
    }

    #[inline]
    fn skip_field(&mut self, key: &'static str) -> Result<()> {
        self.skip_field_inner(key)
    }

    #[inline]
    fn end(self) -> Result<()> {
        self.end_inner()
    }
}

#[doc(hidden)]
pub struct CollectionSerializer<'a, W, O> {
    ser: &'a mut Serializer<W, O>,
    needs_eof: bool,
}

impl<'a, W, O> CollectionSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    #[inline]
    fn end_inner(self) -> Result<()> {
        if self.needs_eof {
            self.ser.writer.write_all(&[0xff]).map_err(|e| e.into())
        } else {
            Ok(())
        }
    }
}

impl<'a, W, O> ser::SerializeSeq for CollectionSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(self) -> Result<()> {
        self.end_inner()
    }
}

impl<'a, W, O> ser::SerializeMap for CollectionSerializer<'a, W, O>
where
    W: Write,
    O: SerializerOptions,
{
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_key<T>(&mut self, key: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        key.serialize(&mut *self.ser)
    }

    #[inline]
    fn serialize_value<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + ser::Serialize,
    {
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(self) -> Result<()> {
        self.end_inner()
    }
}
