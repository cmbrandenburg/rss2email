// FIXME: Delete this file.

use {Error, rmp_serde, std};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::Write;
use std::marker::PhantomData;

struct Reader<'a> {
    cursor: &'a [u8],
}

impl<'a> Reader<'a> {
    fn new(source: &'a [u8]) -> Self {
        Reader { cursor: source }
    }

    fn read_u32(&mut self) -> Result<u32, Error> {
        ReadBytesExt::read_u32::<BigEndian>(&mut self.cursor).map_err(|e| Error::from(("Failed to decode u32", e)))
    }

    fn read_key_kind(&mut self) -> Result<KeyKind, Error> {
        Ok(KeyKind(self.read_u32()
                       .map_err(|e| ("Failed to decode key kind", e))?))
    }

    fn read_str_eof(&mut self) -> Result<&'a str, Error> {
        let s = std::str::from_utf8(self.cursor)
            .map_err(|e| ("Failed to decode string eof", e))?;
        self.cursor = &self.cursor[self.cursor.len()..];
        Ok(s)
    }

    fn read_bytes_eof(&mut self) -> &'a [u8] {
        let b = &self.cursor[..];
        self.cursor = &self.cursor[self.cursor.len()..];
        b
    }
}

struct Writer<W: Write> {
    destination: W,
}

impl<W: Write> Writer<W> {
    fn new(destination: W) -> Self {
        Writer { destination: destination }
    }

    fn write_u32(&mut self, source: u32) -> Result<(), Error> {
        WriteBytesExt::write_u32::<BigEndian>(&mut self.destination, source)
            .map_err(|e| Error::from(("Failed to encode u32", e)))
    }

    fn write_key_kind(&mut self, source: KeyKind) -> Result<(), Error> {
        Ok(self.write_u32(source.0)
               .map_err(|e| ("Failed to encode key kind", e))?)
    }

    fn write_str_eof(&mut self, source: &str) -> Result<(), Error> {
        self.destination
            .write_all(source.as_bytes())
            .map_err(|e| Error::from(("Failed to encode string eof", e)))
    }

    fn write_bytes_eof(&mut self, source: &[u8]) -> Result<(), Error> {
        self.destination
            .write_all(source)
            .map_err(|e| Error::from(("Failed to encode bytes eof", e)))
    }
}


#[derive(Clone, Copy, Debug, PartialEq)]
struct KeyKind(u32);

const KEY_KIND_FEED: KeyKind = KeyKind(1);
const KEY_KIND_ITEM: KeyKind = KeyKind(2);

type KeyKindBytes = [u8; 4];

impl KeyKind {
    fn increment(self) -> Self {
        KeyKind(self.0 + 1)
    }

    fn to_bytes(self) -> KeyKindBytes {
        let mut b = KeyKindBytes::default();
        {
            let mut w = Writer::new(std::io::Cursor::new(&mut b[..]));
            w.write_key_kind(self).unwrap();
        }
        b
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FeedId(u32);

impl FeedId {
    pub fn new(x: u32) -> Self {
        FeedId(x)
    }

    pub fn as_usize(self) -> usize {
        self.0 as usize
    }
}

#[derive(Debug, PartialEq)]
pub struct FeedKey<'a> {
    pub feed_url: &'a str,
}

pub trait FeedSearchMark {}
#[derive(Default)]
pub struct FeedSearchIsEmpty;
impl FeedSearchMark for FeedSearchIsEmpty {}
#[derive(Default)]
pub struct FeedSearchHasFeedUrl;
impl FeedSearchMark for FeedSearchHasFeedUrl {}
#[derive(Default)]
pub struct FeedSearchIsComplete;
impl FeedSearchMark for FeedSearchIsComplete {}

#[derive(Debug, Default, PartialEq)]
pub struct FeedSearch<'a, M: FeedSearchMark> {
    _mark: PhantomData<M>,
    feed_url: Option<&'a str>,
}

impl<'a> FeedKey<'a> {
    pub fn new_search() -> FeedSearch<'a, FeedSearchIsEmpty> {
        FeedSearch::default()
    }

    pub fn min_search() -> KeyKindBytes {
        KEY_KIND_FEED.to_bytes()
    }

    pub fn max_search() -> KeyKindBytes {
        KEY_KIND_FEED.increment().to_bytes()
    }

    pub fn from_bytes(source: &'a [u8]) -> Result<Self, Error> {
        || -> Result<FeedKey, Error> {
            let mut r = Reader::new(source);
            let key_kind = r.read_key_kind()?;
            if key_kind != KEY_KIND_FEED {
                return Err(format!("Got unexpected key kind value: {:?}", key_kind))?;
            }
            let feed_url = r.read_str_eof()?;
            Ok(FeedKey { feed_url: feed_url })
        }()
                .map_err(|e| {
                             Error::from(((format!("Failed to decode database FeedKey (source: {:?})",
                                                   data_dump(source))),
                                          e))
                         })
    }

    fn to_bytes(&self) -> Vec<u8> {
        FeedSearch::<FeedSearchIsComplete> {
                _mark: PhantomData,
                feed_url: Some(self.feed_url),
            }
            .to_bytes()
    }
}

impl<'a> FeedSearch<'a, FeedSearchIsEmpty> {
    pub fn with_feed_url(self, feed_url: &'a str) -> FeedSearch<'a, FeedSearchHasFeedUrl> {
        FeedSearch {
            _mark: PhantomData,
            feed_url: Some(feed_url),
        }
    }
}

impl<'a, M: FeedSearchMark> FeedSearch<'a, M> {
    pub fn to_bytes(self) -> Vec<u8> {
        let mut b = Vec::new();
        {
            let mut w = Writer::new(&mut b);
            w.write_key_kind(KEY_KIND_FEED).unwrap();
            if let Some(feed_url) = self.feed_url {
                w.write_str_eof(feed_url).unwrap();
            }
        }
        b
    }
}

#[derive(Debug, PartialEq)]
pub struct FeedValue {
    pub feed_id: FeedId,
    pub when_added: std::time::SystemTime,
}

#[derive(Debug, Deserialize, Serialize)]
struct FeedValueSerializable {
    feed_id: FeedId,
    when_added: (u64, u32),
}

impl FeedValue {
    pub fn from_bytes(source: &[u8]) -> Result<Self, Error> {

        let x: FeedValueSerializable = rmp_serde::from_slice(source)
            .map_err(|e| {
                         (format!("Failed to decode database FeedValue (source: {:?})",
                                  data_dump(source)),
                          e)
                     })?;

        Ok(FeedValue {
               feed_id: x.feed_id,
               when_added: std::time::UNIX_EPOCH + std::time::Duration::new(x.when_added.0, x.when_added.1),
           })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let d = self.when_added
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let x = FeedValueSerializable {
            feed_id: self.feed_id,
            when_added: (d.as_secs(), d.subsec_nanos()),
        };
        rmp_serde::to_vec(&x).unwrap()
    }
}

pub trait ItemSearchMark {}
#[derive(Default)]
pub struct ItemSearchIsEmpty;
impl ItemSearchMark for ItemSearchIsEmpty {}
#[derive(Default)]
pub struct ItemSearchHasFeedUrl;
impl ItemSearchMark for ItemSearchHasFeedUrl {}
#[derive(Default)]
pub struct ItemSearchHasItemId;
impl ItemSearchMark for ItemSearchHasItemId {}

#[derive(Debug, PartialEq)]
pub struct ItemKey<'a> {
    pub feed_id: FeedId,
    pub item_id: &'a str,
}

#[derive(Debug, Default, PartialEq)]
pub struct ItemSearch<'a, M: ItemSearchMark> {
    _mark: PhantomData<M>,
    feed_id: Option<FeedId>,
    item_id: Option<&'a str>,
}

impl<'a> ItemKey<'a> {
    pub fn new_search() -> ItemSearch<'a, ItemSearchIsEmpty> {
        ItemSearch::default()
    }

    pub fn from_bytes(source: &'a [u8]) -> Result<Self, Error> {
        || -> Result<ItemKey, Error> {
            let mut r = Reader::new(source);
            let key_kind = r.read_key_kind()?;
            if key_kind != KEY_KIND_ITEM {
                return Err(format!("Got unexpected key kind value: {:?}", key_kind))?;
            }
            let feed_id = FeedId::new(r.read_u32()?);
            let item_id = r.read_str_eof()?;
            Ok(ItemKey {
                   feed_id: feed_id,
                   item_id: item_id,
               })
        }()
                .map_err(|e| {
                             Error::from(((format!("Failed to decode database ItemKey (source: {:?})",
                                                   data_dump(source))),
                                          e))
                         })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        ItemSearch::<ItemSearchIsEmpty> {
                _mark: PhantomData,
                feed_id: Some(self.feed_id),
                item_id: Some(self.item_id),
            }
            .to_bytes()
    }
}

impl<'a> ItemSearch<'a, ItemSearchIsEmpty> {
    pub fn with_feed_id(self, feed_id: FeedId) -> ItemSearch<'a, ItemSearchHasItemId> {
        ItemSearch {
            _mark: PhantomData,
            feed_id: Some(feed_id),
            item_id: None,
        }
    }
}

impl<'a, M: ItemSearchMark> ItemSearch<'a, M> {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::new();
        {
            let mut w = Writer::new(&mut b);
            w.write_key_kind(KEY_KIND_ITEM).unwrap();
            if let Some(feed_id) = self.feed_id {
                w.write_u32(feed_id.0).unwrap();
                if let Some(item_id) = self.item_id {
                    w.write_str_eof(item_id).unwrap();
                }
            }
        }
        b
    }
}

#[derive(Debug, PartialEq)]
pub struct ItemValue {
    pub when_added: std::time::SystemTime,
}

#[derive(Debug, Deserialize, Serialize)]
struct ItemValueSerializable {
    when_added: (u64, u32),
}

impl ItemValue {
    pub fn from_bytes(source: &[u8]) -> Result<Self, Error> {

        let x: ItemValueSerializable = rmp_serde::from_slice(source)
            .map_err(|e| {
                         (format!("Failed to decode database ItemValue (source: {:?})",
                                  data_dump(source)),
                          e)
                     })?;

        Ok(ItemValue { when_added: std::time::UNIX_EPOCH + std::time::Duration::new(x.when_added.0, x.when_added.1) })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let d = self.when_added
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        let x = ItemValueSerializable { when_added: (d.as_secs(), d.subsec_nanos()) };
        rmp_serde::to_vec(&x).unwrap()
    }
}

// Helper method for displaying an appropriate dump of raw binary data.
fn data_dump(d: &[u8]) -> String {
    if d.len() < 100 {
        return format!("{:?}", d);
    }

    format!("(truncated, was size {}) {:?}", d.len(), &d[..100])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_u32() {
        let buffer = &mut [0; 4][..];
        {
            let mut w = Writer::new(&mut buffer[..]);
            w.write_u32(0x01020304).unwrap();
        }
        let mut r = Reader::new(buffer);
        assert_eq!(r.read_u32().unwrap(), 0x01020304);
    }

    #[test]
    fn read_write_key_kind() {
        let buffer = &mut [0; 4][..];
        {
            let mut w = Writer::new(&mut buffer[..]);
            w.write_key_kind(KeyKind(0x01020304)).unwrap();
        }
        let mut r = Reader::new(buffer);
        assert_eq!(r.read_key_kind().unwrap(), KeyKind(0x01020304));
    }

    #[test]
    fn read_write_str_eof() {
        let buffer = &mut [0; 5][..];
        {
            let mut w = Writer::new(&mut buffer[..]);
            w.write_str_eof("hello").unwrap();
        }
        let mut r = Reader::new(buffer);
        assert_eq!(r.read_str_eof().unwrap(), "hello");
    }

    #[test]
    fn read_write_bytes_eof() {
        let buffer = &mut [0; 5][..];
        {
            let mut w = Writer::new(&mut buffer[..]);
            w.write_bytes_eof(b"hello").unwrap();
        }
        let mut r = Reader::new(buffer);
        assert_eq!(r.read_bytes_eof(), b"hello");
    }

    #[test]
    fn feed_key_converts_to_and_from_bytes() {
        let source = FeedKey { feed_url: "https://xkcd.com/rss.xml" };
        let b = source.to_bytes();
        let destination = FeedKey::from_bytes(&b).unwrap();
        assert_eq!(source, destination);
    }

    #[test]
    fn feed_value_converts_to_and_from_bytes() {
        let source = FeedValue {
            feed_id: FeedId::new(42),
            when_added: std::time::SystemTime::now(),
        };
        let b = source.to_bytes();
        let destination = FeedValue::from_bytes(&b).unwrap();
        assert_eq!(source, destination);
    }

    #[test]
    fn item_key_converts_to_and_from_bytes() {
        let source = ItemKey {
            feed_id: FeedId::new(42),
            item_id: "https://xkcd.com/1850/",
        };
        let b = source.to_bytes();
        let destination = ItemKey::from_bytes(&b).unwrap();
        assert_eq!(source, destination);
    }

    #[test]
    fn item_value_converts_to_and_from_bytes() {
        let source = ItemValue { when_added: std::time::SystemTime::now() };
        let b = source.to_bytes();
        let destination = ItemValue::from_bytes(&b).unwrap();
        assert_eq!(source, destination);
    }
}
