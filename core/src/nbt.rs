//! A small reader of uncompressed binary NBT (big-endian, as region files store chunks).
//! Only what reading a chunk needs: every tag type is parsed, nothing is written.
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    End,
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(HashMap<String, Tag>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    pub fn get(&self, key: &str) -> Option<&Tag> {
        match self {
            Tag::Compound(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_list(&self) -> &[Tag] {
        match self {
            Tag::List(l) => l,
            _ => &[],
        }
    }
    pub fn as_longs(&self) -> Option<&[i64]> {
        match self {
            Tag::LongArray(l) => Some(l),
            _ => None,
        }
    }
    /// Any integer tag as an i64 (a section's `Y` is a byte).
    pub fn as_int(&self) -> Option<i64> {
        match *self {
            Tag::Byte(v) => Some(i64::from(v)),
            Tag::Short(v) => Some(i64::from(v)),
            Tag::Int(v) => Some(i64::from(v)),
            Tag::Long(v) => Some(v),
            _ => None,
        }
    }
}

/// Nesting deeper than this is refused (a chunk needs about six levels).
const MAX_DEPTH: usize = 512;

/// Reads the root tag (a named compound; the name is dropped).
pub fn read(bytes: &[u8]) -> Result<Tag, String> {
    let mut r = Reader { b: bytes, at: 0 };
    let kind = r.u8()?;
    if kind != 10 {
        return Err(format!("root tag is type {kind}, not a compound"));
    }
    let _name = r.string()?;
    r.payload(10, 0)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.b.len()).ok_or("truncated NBT")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> Result<i16, String> {
        Ok(i16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn len(&mut self) -> Result<usize, String> {
        let n = self.i32()?;
        usize::try_from(n).map_err(|_| format!("negative length {n}"))
    }
    fn string(&mut self) -> Result<String, String> {
        let n = self.i16()? as u16 as usize;
        // Modified UTF-8; plain UTF-8 for everything a chunk holds, lossy otherwise.
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn payload(&mut self, kind: u8, depth: usize) -> Result<Tag, String> {
        if depth > MAX_DEPTH {
            return Err("NBT nested too deep".into());
        }
        Ok(match kind {
            0 => Tag::End,
            1 => Tag::Byte(self.u8()? as i8),
            2 => Tag::Short(self.i16()?),
            3 => Tag::Int(self.i32()?),
            4 => Tag::Long(self.i64()?),
            5 => Tag::Float(f32::from_bits(self.i32()? as u32)),
            6 => Tag::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            8 => Tag::String(self.string()?),
            9 => {
                let inner = self.u8()?;
                let n = self.len()?;
                if n > self.b.len() {
                    return Err("list longer than the data".into());
                }
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(self.payload(inner, depth + 1)?);
                }
                Tag::List(list)
            }
            10 => {
                let mut map = HashMap::new();
                loop {
                    let k = self.u8()?;
                    if k == 0 {
                        break;
                    }
                    let name = self.string()?;
                    let v = self.payload(k, depth + 1)?;
                    map.insert(name, v);
                }
                Tag::Compound(map)
            }
            11 => {
                let n = self.len()?;
                let raw = self.take(n.checked_mul(4).ok_or("int array too long")?)?;
                Tag::IntArray(raw.chunks_exact(4).map(|c| i32::from_be_bytes(c.try_into().unwrap())).collect())
            }
            12 => {
                let n = self.len()?;
                let raw = self.take(n.checked_mul(8).ok_or("long array too long")?)?;
                Tag::LongArray(raw.chunks_exact(8).map(|c| i64::from_be_bytes(c.try_into().unwrap())).collect())
            }
            k => return Err(format!("unknown NBT tag type {k}")),
        })
    }
}

/// A writer for tests: builds the NBT a region file would hold.
#[cfg(test)]
pub mod write {
    use super::Tag;

    pub fn root(tag: &Tag) -> Vec<u8> {
        let mut out = vec![10, 0, 0];
        payload(&mut out, tag);
        out
    }

    fn kind(tag: &Tag) -> u8 {
        match tag {
            Tag::End => 0,
            Tag::Byte(_) => 1,
            Tag::Short(_) => 2,
            Tag::Int(_) => 3,
            Tag::Long(_) => 4,
            Tag::Float(_) => 5,
            Tag::Double(_) => 6,
            Tag::ByteArray(_) => 7,
            Tag::String(_) => 8,
            Tag::List(_) => 9,
            Tag::Compound(_) => 10,
            Tag::IntArray(_) => 11,
            Tag::LongArray(_) => 12,
        }
    }

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend_from_slice(&(s.len() as u16).to_be_bytes());
        out.extend_from_slice(s.as_bytes());
    }

    fn payload(out: &mut Vec<u8>, tag: &Tag) {
        match tag {
            Tag::End => {}
            Tag::Byte(v) => out.push(*v as u8),
            Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
            Tag::ByteArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                out.extend(v.iter().map(|&b| b as u8));
            }
            Tag::String(s) => string(out, s),
            Tag::List(l) => {
                out.push(l.first().map_or(0, kind));
                out.extend_from_slice(&(l.len() as i32).to_be_bytes());
                for t in l {
                    payload(out, t);
                }
            }
            Tag::Compound(m) => {
                for (k, v) in m {
                    out.push(kind(v));
                    string(out, k);
                    payload(out, v);
                }
                out.push(0);
            }
            Tag::IntArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
            Tag::LongArray(v) => {
                out.extend_from_slice(&(v.len() as i32).to_be_bytes());
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }

    pub fn compound(entries: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::write::{compound, root};
    use super::*;

    #[test]
    fn every_type_round_trips() {
        let tag = compound(vec![
            ("b", Tag::Byte(-3)),
            ("s", Tag::Short(300)),
            ("i", Tag::Int(-70000)),
            ("l", Tag::Long(1 << 40)),
            ("f", Tag::Float(1.5)),
            ("d", Tag::Double(-2.25)),
            ("ba", Tag::ByteArray(vec![1, -1])),
            ("str", Tag::String("minecraft:stone".into())),
            ("list", Tag::List(vec![Tag::Int(1), Tag::Int(2)])),
            ("empty", Tag::List(vec![])),
            ("ia", Tag::IntArray(vec![5, -5])),
            ("la", Tag::LongArray(vec![i64::MIN, 7])),
            ("c", compound(vec![("Y", Tag::Byte(-4))])),
        ]);
        assert_eq!(read(&root(&tag)).unwrap(), tag);
        assert_eq!(tag.get("c").and_then(|c| c.get("Y")).and_then(Tag::as_int), Some(-4));
    }

    #[test]
    fn truncated_and_bad_input_is_an_error() {
        let bytes = root(&compound(vec![("la", Tag::LongArray(vec![1, 2, 3]))]));
        assert!(read(&bytes[..bytes.len() - 5]).is_err());
        assert!(read(&[8, 0, 0]).is_err());
        assert!(read(&[10, 0, 0, 13, 0, 1, b'x']).is_err());
    }
}
