// SPDX-License-Identifier: Apache-2.0
//! Text transmit streams translated through their effective schemas.

use super::{base, own_definition, read_definition, BaseVersion, DataType, SchemaPlan};
use cadmpeg_core::{decode::View, CodecError};
use std::collections::BTreeMap;

const MAX_BYTES: usize = 256 * 1024 * 1024;
const MAX_ELEMENTS: u32 = 4_000_000;
const MAX_RECORDS: usize = 1_000_000;

fn bad(message: &str) -> CodecError {
    CodecError::Malformed(format!("Parasolid text: {message}"))
}

struct Text {
    bytes: Vec<u8>,
    at: usize,
    spaces: u8,
    escaped: bool,
}

impl Text {
    fn new(input: &[u8]) -> Result<Self, CodecError> {
        if input.len() > MAX_BYTES {
            return Err(bad("input exceeds size limit"));
        }
        let start = if input.starts_with(b"**") {
            let marker = b"**END_OF_HEADER";
            let at = input
                .get(..input.len().min(65536))
                .ok_or_else(|| bad("header window"))?
                .windows(marker.len())
                .position(|w| w == marker)
                .ok_or_else(|| bad("missing common header terminator"))?;
            at + input
                .get(at..)
                .and_then(|s| s.iter().position(|b| *b == b'\n'))
                .ok_or_else(|| bad("incomplete common header"))?
                + 1
        } else {
            0
        };
        let mut bytes = Vec::new();
        for line in input
            .get(start..)
            .ok_or_else(|| bad("invalid header"))?
            .split(|b| *b == b'\n')
        {
            let end = line
                .iter()
                .rposition(|b| *b != b' ' && *b != b'\r')
                .map_or(0, |i| i + 1);
            bytes.extend(
                line.get(..end)
                    .ok_or_else(|| bad("invalid line"))?
                    .iter()
                    .filter(|b| **b != b'\r'),
            );
        }
        Ok(Self {
            bytes,
            at: 0,
            spaces: 0,
            escaped: false,
        })
    }

    fn raw(&mut self) -> Result<u8, CodecError> {
        let byte = *self
            .bytes
            .get(self.at)
            .ok_or_else(|| bad("unexpected end of input"))?;
        self.at += 1;
        Ok(byte)
    }

    fn character(&mut self) -> Result<u8, CodecError> {
        if self.spaces > 0 {
            self.spaces -= 1;
            return Ok(b' ');
        }
        let byte = self.raw()?;
        if byte != b'\\' || !self.escaped {
            return Ok(byte);
        }
        match self.raw()? {
            b'0' => Ok(0),
            b'n' => Ok(b'\r'),
            b'r' => Ok(b'\n'),
            b'\\' => Ok(b'\\'),
            b'9' => {
                self.spaces = 8;
                Ok(b' ')
            }
            _ => Err(bad("invalid character escape")),
        }
    }

    fn token(&mut self) -> Result<&str, CodecError> {
        if self.spaces != 0 {
            return Err(bad("space run crosses a character field"));
        }
        let start = self.at;
        if self.bytes.get(start) == Some(&b'?') {
            self.at += 1;
            return Ok("?");
        }
        while self.bytes.get(self.at).is_some_and(|b| *b != b' ') {
            self.at += 1;
            if self.at - start > 80 {
                return Err(bad("numeric token exceeds limit"));
            }
        }
        let end = self.at;
        if self.bytes.get(self.at) == Some(&b' ') {
            self.at += 1;
        }
        std::str::from_utf8(
            self.bytes
                .get(start..end)
                .ok_or_else(|| bad("invalid token"))?,
        )
        .map_err(|_| bad("non-ASCII number"))
    }

    fn unsigned(&mut self) -> Result<u32, CodecError> {
        self.token()?
            .parse()
            .map_err(|_| bad("invalid unsigned integer"))
    }

    fn short(&mut self) -> Result<u16, CodecError> {
        u16::try_from(self.unsigned()?).map_err(|_| bad("short integer overflow"))
    }

    fn byte(&mut self) -> Result<u8, CodecError> {
        u8::try_from(self.unsigned()?).map_err(|_| bad("byte overflow"))
    }

    fn string(&mut self, limit: usize) -> Result<Vec<u8>, CodecError> {
        let count = usize::try_from(self.unsigned()?).map_err(|_| bad("string length overflow"))?;
        if count > limit {
            return Err(bad("string exceeds limit"));
        }
        (0..count).map(|_| self.character()).collect()
    }

    fn logical(&mut self) -> Result<u8, CodecError> {
        match self.character()? {
            b'F' => Ok(0),
            b'T' => Ok(1),
            _ => Err(bad("invalid logical")),
        }
    }

    fn positive(&mut self, out: &mut Vec<u8>) -> Result<(), CodecError> {
        let value = self
            .unsigned()?
            .checked_add(1)
            .ok_or_else(|| bad("index overflow"))?;
        crate::pointer::write(out, value).ok_or_else(|| bad("index exceeds encoding range"))
    }

    fn short_string(&mut self, out: &mut Vec<u8>) -> Result<(), CodecError> {
        let value = self.string(255)?;
        out.push(u8::try_from(value.len()).map_err(|_| bad("short string overflow"))?);
        out.extend(value);
        Ok(())
    }

    fn descriptor(&mut self, out: &mut Vec<u8>) -> Result<(), CodecError> {
        self.short_string(out)?;
        let class = self.short()?;
        out.extend(class.to_be_bytes());
        let count = self.unsigned()?;
        crate::pointer::write(
            out,
            count.checked_add(1).ok_or_else(|| bad("extent overflow"))?,
        )
        .ok_or_else(|| bad("extent exceeds encoding range"))?;
        if class == 0 {
            self.short_string(out)?;
        }
        if count == 1 {
            out.push(self.logical()?);
        }
        Ok(())
    }

    fn definition(
        &mut self,
        out: &mut Vec<u8>,
        version: BaseVersion,
        kind: u16,
    ) -> Result<SchemaPlan, CodecError> {
        let count = self.byte()?;
        out.push(count);
        let start = out.len();
        if count != 255 {
            if base::fields(version, kind).is_some() {
                let mut finished = false;
                for _ in 0..1024 {
                    let op = self.character()?;
                    out.push(op);
                    match op {
                        b'Z' => {
                            finished = true;
                            break;
                        }
                        b'C' | b'D' => (),
                        b'I' | b'A' => self.descriptor(out)?,
                        _ => return Err(bad("invalid schema edit")),
                    }
                }
                if !finished {
                    return Err(bad("schema edit exceeds operation limit"));
                }
            } else {
                self.short_string(out)?;
                self.short_string(out)?;
                for _ in 0..count {
                    self.descriptor(out)?;
                }
            }
        }
        let mut view =
            View::over_retained(out.get(start..).ok_or_else(|| bad("invalid definition"))?);
        read_definition(&mut view, version, kind, count)?
            .ok_or_else(|| bad("invalid effective schema"))
    }

    fn scalar(&mut self, out: &mut Vec<u8>, ty: DataType) -> Result<(), CodecError> {
        match ty {
            DataType::Char => out.push(self.character()?),
            DataType::Logical => out.push(self.logical()?),
            DataType::Byte => out.push(self.byte()?),
            DataType::Pointer => self.positive(out)?,
            DataType::Short | DataType::Word | DataType::Integer => {
                let token = self.token()?;
                let value: i64 = if token == "?" {
                    -32764
                } else {
                    token.parse().map_err(|_| bad("invalid integer"))?
                };
                if ty == DataType::Integer {
                    if !(-2_147_483_648..=4_294_967_295).contains(&value) {
                        return Err(bad("integer overflow"));
                    }
                    let bytes = value.to_be_bytes();
                    out.extend(bytes.get(4..).ok_or_else(|| bad("integer encoding"))?);
                } else {
                    if !(-32768..=65535).contains(&value) {
                        return Err(bad("short integer overflow"));
                    }
                    let bytes = value.to_be_bytes();
                    out.extend(bytes.get(6..).ok_or_else(|| bad("short encoding"))?);
                }
            }
            DataType::Unestablished(_) => return Err(bad("unsupported scalar type")),
            _ => {
                let width = match ty {
                    DataType::Interval => 2,
                    DataType::Vector | DataType::Hvec => 3,
                    DataType::Box => 6,
                    _ => 1,
                };
                for component in 0..width {
                    let token = self.token()?;
                    if token == "?" {
                        let repeat =
                            if component == 0 && matches!(ty, DataType::Vector | DataType::Hvec) {
                                width
                            } else {
                                1
                            };
                        for _ in 0..repeat {
                            out.extend(super::fields::NULL_REAL);
                        }
                        if repeat > 1 {
                            break;
                        }
                    } else {
                        let value: f64 = token.parse().map_err(|_| bad("invalid real"))?;
                        if !value.is_finite() {
                            return Err(bad("nonfinite real"));
                        }
                        out.extend(value.to_be_bytes());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Translate a text XT file into a neutral binary stream for [`super::PrimaryRecords`].
///
/// Accepts base schemas 11004 and 13006 and embedded edits over those bases.
/// Returns an error for unsupported schemas, user fields, malformed or oversized input.
pub fn to_neutral_binary(input: &[u8]) -> Result<Vec<u8>, CodecError> {
    let mut text = Text::new(input)?;
    if text.raw()? != b'T' {
        return Err(bad("expected text format flag"));
    }
    let description = text.string(65535)?;
    let schema = text.string(255)?;
    let schema_text = std::str::from_utf8(&schema).map_err(|_| bad("invalid schema name"))?;
    let parts: Vec<_> = schema_text.split('_').collect();
    if parts.first() != Some(&"SCH") || !(3..=4).contains(&parts.len()) {
        return Err(bad("invalid schema name"));
    }
    let version = match parts.last().copied() {
        Some("11004") => BaseVersion::V11004,
        Some("13006") => BaseVersion::V13006,
        _ => return Err(bad("unsupported base schema")),
    };
    text.escaped = parts
        .get(1)
        .and_then(|s| s.parse::<u32>().ok())
        .is_some_and(|v| v >= 1_210_000);
    let embedded = parts.len() == 4;
    let maximum = if embedded {
        text.short()?
    } else {
        base::maximum(version)
    };
    if text.unsigned()? != 0 {
        return Err(bad("user fields are unsupported"));
    }
    let mut out = b"PS".to_vec();
    out.extend(
        u32::try_from(description.len())
            .map_err(|_| bad("description overflow"))?
            .to_be_bytes(),
    );
    out.extend(description);
    out.extend(
        u32::try_from(schema.len())
            .map_err(|_| bad("schema overflow"))?
            .to_be_bytes(),
    );
    out.extend(schema);
    if embedded {
        out.extend(maximum.to_be_bytes());
    }
    out.extend(0u32.to_be_bytes());
    let mut definitions = BTreeMap::new();
    for _ in 0..MAX_RECORDS {
        let kind = text.short()?;
        out.extend(kind.to_be_bytes());
        if kind == 1 {
            if text.unsigned()? != 0 {
                return Err(bad("invalid terminator"));
            }
            out.extend(1u16.to_be_bytes());
            if text.at != text.bytes.len() {
                return Err(bad("data after terminator"));
            }
            return Ok(out);
        }
        if kind < 2 || kind > maximum {
            return Err(bad("node type outside schema bounds"));
        }
        if let std::collections::btree_map::Entry::Vacant(entry) = definitions.entry(kind) {
            let plan = if embedded {
                text.definition(&mut out, version, kind)?
            } else {
                own_definition(
                    kind,
                    base::fields(version, kind).ok_or_else(|| bad("unknown node type"))?,
                )?
            };
            entry.insert(plan);
        }
        let plan = definitions
            .get(&kind)
            .ok_or_else(|| bad("missing node schema"))?;
        let variable = if plan.fields.iter().any(|field| field.n_elements == 1) {
            let count = text.unsigned()?;
            if count > MAX_ELEMENTS {
                return Err(bad("variable length exceeds limit"));
            }
            out.extend(count.to_be_bytes());
            count
        } else {
            0
        };
        let identity = text.unsigned()?;
        if identity == 0 {
            return Err(bad("null node identity"));
        }
        crate::pointer::write(
            &mut out,
            identity
                .checked_add(1)
                .ok_or_else(|| bad("identity overflow"))?,
        )
        .ok_or_else(|| bad("identity exceeds encoding range"))?;
        for field in &plan.fields {
            if !field.transmit {
                continue;
            }
            let count = if field.n_elements == 1 {
                variable
            } else {
                field.n_elements.max(1)
            };
            if count > MAX_ELEMENTS {
                return Err(bad("field extent exceeds limit"));
            }
            for _ in 0..count {
                if out.len() > MAX_BYTES - 48 {
                    return Err(bad("output exceeds size limit"));
                }
                text.scalar(&mut out, field.data_type)?;
            }
        }
    }
    Err(bad("record count exceeds limit"))
}

#[cfg(test)]
mod tests;
