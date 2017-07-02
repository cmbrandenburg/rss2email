use {Error, FakeDebug, lmdb, lmdb_sys, rmp_serde, std};
use feed::{Fetcher, Sender};
use futures;
use lmdb::{Cursor, Transaction};
use log::{LogKind, LogLevel, Logger};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const DB_FEED: &str = "feed";
const DB_ITEM: &str = "item";

#[derive(Debug, Default)]
pub struct FetchAndSendOptions {
    pub feed_urls: Option<HashSet<String>>,
}

impl FetchAndSendOptions {
    fn should_fetch(&self, feed_url: &str) -> bool {
        if let Some(ref m) = self.feed_urls {
            return m.contains(feed_url);
        }
        return true;
    }
}

#[derive(Debug)]
pub struct Model {
    environment: FakeDebug<lmdb::Environment>, // TODO: Need newer lmdb crate for Debug impl
    db_feed: lmdb::Database,
    db_item: lmdb::Database,
    path: PathBuf,
}

impl Model {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self, Error> {

        let path = path.as_ref();

        match std::fs::metadata(path) {
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err((
                format!("Failed to obtain file metadata (path: {:?})", path),
                e,
            ))?,
            Ok(..) => return Err(format!("Database already exists (path: {:?})", path))?,
        }

        Self::open_impl(path)
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {

        let path = path.as_ref();

        match std::fs::metadata(path) {
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => return Err(format!(
                "Database does not exist (path: {:?})",
                path
            ))?,
            Err(e) => return Err((
                format!("Failed to obtain file metadata (path: {:?})", path),
                e,
            ))?,
            Ok(..) => {}
        }

        Self::open_impl(path)
    }

    fn open_impl(path: &Path) -> Result<Self, Error> {

        let environment = lmdb::Environment::new()
            .set_flags(lmdb::NO_SUB_DIR)
            .set_max_dbs(2)
            .open(path)
            .map_err(|e| {
                (
                    format!("Failed to open LMDB environment (path: {:?})", path),
                    e,
                )
            })?;

        let open_db = |environment: &lmdb::Environment, db_name: &str| -> Result<lmdb::Database, Error> {
            let flags = lmdb::DatabaseFlags::empty();
            environment.create_db(Some(db_name), flags).map_err(|e| {
                Error::from((
                    format!(
                        "Failed to open LMDB database (path: {:?}, database: {:?}, flags = 0x{:x})",
                        path,
                        db_name,
                        flags.bits()
                    ),
                    e,
                ))
            })
        };

        Ok(Model {
            path: PathBuf::from(path),
            db_feed: open_db(&environment, DB_FEED)?,
            db_item: open_db(&environment, DB_ITEM)?,
            environment: FakeDebug(environment),
        })
    }

    fn begin_rw_transaction(&self) -> Result<lmdb::RwTransaction, Error> {
        self.environment.begin_rw_txn().map_err(|e| {
            Error::from((
                format!(
                    "Failed to begin LMDB read-write transaction (path: {:?})",
                    self.path
                ),
                e,
            ))
        })
    }

    fn begin_ro_transaction(&self) -> Result<lmdb::RoTransaction, Error> {
        self.environment.begin_ro_txn().map_err(|e| {
            Error::from((
                format!(
                    "Failed to begin LMDB read-only transaction (path: {:?})",
                    self.path
                ),
                e,
            ))
        })
    }

    fn commit_transaction(&self, tx: lmdb::RwTransaction) -> Result<(), Error> {
        tx.commit().map_err(|e| {
            Error::from((
                format!(
                    "Failed to commit database transaction (path: {:?})",
                    self.path
                ),
                e,
            ))
        })
    }

    fn delete_all_items_for_feed(&self, tx: &mut lmdb::RwTransaction, feed_url: &str) -> Result<(), Error> {

        let mut cursor = tx.open_rw_cursor(self.db_item).map_err(|e| {
            (
                format!("Failed to obtain read-write cursor to {:?} table", DB_ITEM),
                e,
            )
        })?;

        let search_bytes = &DbItemSearch {
            feed_url: feed_url,
            item_id: None,
        }.to_bytes();

        let mut key_bytes = match cursor.get(Some(&search_bytes), None, lmdb_sys::MDB_SET_RANGE) {
            Err(lmdb::Error::NotFound) => return Ok(()), // nothing to do
            Err(e) => Err((
                format!(
                    "Failed to position cursor to first feed item in {:?} table",
                    DB_ITEM
                ),
                e,
            ))?,
            Ok((None, _)) => unreachable!(),
            Ok((Some(x), _)) => x,
        };

        while key_bytes.starts_with(search_bytes) {
            cursor.del(lmdb::WriteFlags::empty()).map_err(|e| {
                (
                    format!("Failed to decode feed item from {:?} table", DB_ITEM),
                    e,
                )
            })?;

            key_bytes = match cursor.get(None, None, lmdb_sys::MDB_NEXT) {
                Ok(x) => x.0.unwrap(),
                Err(lmdb::Error::NotFound) => break,
                Err(e) => Err((
                    format!(
                        "Failed to advance cursor position within {:?} table",
                        DB_ITEM
                    ),
                    e,
                ))?,
            };
        }

        Ok(())
    }

    pub fn for_each_feed<F>(&self, mut callback: F) -> Result<(), Error>
    where
        F: FnMut(&str) -> Result<(), Error>,
    {
        let tx = self.begin_ro_transaction()?;

        for (key_bytes, _value_bytes) in
            tx.open_ro_cursor(self.db_feed)
                .map_err(|e| {
                    (format!("Failed to obtain cursor to {:?} table", DB_FEED), e)
                })?
                .iter()
        {
            let key = DbFeedKey::from_bytes(key_bytes)?;
            callback(key.feed_url)?;
        }

        Ok(())
    }

    pub fn add_feed(&self, feed_url: &str) -> Result<(), Error> {

        let mut tx = self.begin_rw_transaction()?;

        let key = DbFeedKey { feed_url: feed_url };
        let key_bytes = key.to_bytes();

        match tx.get(self.db_feed, &key_bytes) {
            Err(lmdb::Error::NotFound) => {}
            Err(e) => return Err((
                format!(
                    "Failed to look up feed in {:?} table (feed URL: {:?})",
                    DB_FEED,
                    feed_url
                ),
                e,
            ))?,
            Ok(..) => return Err((format!("Feed already exists (feed URL: {:?})", feed_url)))?,
        }

        // Normally there won't be any feed items for a nonexistent feed, but,
        // by deleting them anyway, we guarantee the database is in a valid
        // state.

        self.delete_all_items_for_feed(&mut tx, feed_url)?;

        let now = std::time::SystemTime::now();

        let value = DbFeedValue { when_added: now };
        let value_bytes = value.to_bytes();

        tx.put(
            self.db_feed,
            &key_bytes,
            &value_bytes,
            lmdb::WriteFlags::empty(),
        ).map_err(|e| {
                (
                    format!(
                        "Failed to add feed to {:?} table (feed URL: {:?})",
                        DB_FEED,
                        feed_url
                    ),
                    e,
                )
            })?;

        self.commit_transaction(tx)
    }

    pub fn remove_feed(&self, feed_url: &str) -> Result<(), Error> {

        let mut tx = self.begin_rw_transaction()?;

        let key = DbFeedKey { feed_url: feed_url };
        let key_bytes = key.to_bytes();

        match tx.del(self.db_feed, &key_bytes, None) {
            Err(lmdb::Error::NotFound) => return Err(format!("Feed does not exist (feed URL: {:?})", feed_url))?,
            Err(e) => return Err((
                format!(
                    "Failed to delete feed from {:?} table (feed URL: {:?})",
                    DB_FEED,
                    feed_url
                ),
                e,
            ))?,
            Ok(..) => {}
        }

        self.delete_all_items_for_feed(&mut tx, feed_url)?;
        self.commit_transaction(tx)
    }

    pub fn fetch_and_send_feeds<F, S>(
        &self,
        logger: Arc<Logger>,
        fetcher: F,
        sender: &S,
        options: &FetchAndSendOptions,
    ) -> Result<(), Error>
    where
        F: Fetcher,
        S: Sender,
    {
        // Even though this is a long-running operation, we do it within a
        // single transaction so that there are no race conditions with other
        // processes mucking around with the database.

        let mut tx = self.begin_rw_transaction()?;

        {
            let now = std::time::SystemTime::now();

            // First collect all feeds to fetch. We must collect these in memory
            // so that we're then free to modify the database to update the
            // feeds' states. I.e., we can't read the feeds from the database while
            // simultaneously modifying the database.

            let feeds_to_fetch = tx.open_ro_cursor(self.db_feed)
                .map_err(|e| {
                    (format!("Failed to obtain cursor to {:?} table", DB_FEED), e)
                })?
                .iter()
                .map(|(key_bytes, _value_bytes)| -> Result<String, Error> {
                    Ok(String::from(DbFeedKey::from_bytes(key_bytes)?.feed_url))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter(|feed_url| options.should_fetch(feed_url))
                .collect::<Vec<_>>();

            if let Some(ref filter) = options.feed_urls {
                for f in filter {
                    if feeds_to_fetch.iter().all(|g| f != g) {
                        return Err(format!("Feed URL {} does not exist in the database", f))?;
                    }
                }
            }

            // Pump the fetcher for the feeds it receives. Send each item we
            // receive. Update the database state as we go.

            let mut spawn = futures::executor::spawn(fetcher.fetch(logger.clone(), feeds_to_fetch));
            while let Some(fetch_result) = spawn.wait_stream() {

                let feed = match fetch_result {
                    Err(e) => {
                        logger.log(LogLevel::Important, LogKind::Error, e);
                        break;;
                    }
                    Ok(x) => x,
                };

                // Don't send items we've previously received. But update the
                // `when_last_fetched` value in the database.

                for item in &feed.items {

                    let item_key_bytes = DbItemKey {
                        feed_url: &feed.meta.feed_url,
                        item_id: &item.id,
                    }.to_bytes();

                    let item_value = match tx.get(self.db_item, &item_key_bytes) {
                        Ok(item_value_bytes) => {
                            let mut item_value = DbItemValue::from_bytes(item_value_bytes)?;
                            item_value.when_last_fetched = now;
                            Some(item_value)
                        }
                        Err(lmdb::Error::NotFound) => None,
                        Err(e) => Err((
                            format!(
                                "Failed to look up feed item in {:?} table",
                                DB_ITEM
                            ),
                            e,
                        ))?,
                    };

                    if let Some(item_value) = item_value {
                        tx.put(
                            self.db_item,
                            &item_key_bytes,
                            &item_value.to_bytes(),
                            lmdb::WriteFlags::empty(),
                        ).map_err(|e| {
                                (
                                    format!("Failed to update feed item in {:?} table", DB_ITEM),
                                    e,
                                )
                            })?;
                        continue;
                    }

                    // Send (the new feed item).

                    match sender.send(&logger, &feed.meta, &item) {
                        Err(e) => {
                            logger.log(
                                LogLevel::Important,
                                LogKind::Error,
                                format!(
                                    "An error occurred while sending (feed id = {}): {}",
                                    item.id,
                                    e
                                ),
                            );
                            break;
                        }
                        Ok(..) => {

                            let item_value_bytes = DbItemValue {
                                when_first_fetched: now,
                                when_last_fetched: now,
                            }.to_bytes();

                            tx.put(
                                self.db_item,
                                &item_key_bytes,
                                &item_value_bytes,
                                lmdb::WriteFlags::empty(),
                            ).map_err(|e| {
                                    (
                                        format!(
                                            "Failed to add feed item to {:?} table (feed URL: {:?}, feed item = {:?})",
                                            DB_ITEM,
                                            feed.meta.feed_url,
                                            item
                                        ),
                                        e,
                                    )
                                })?;
                        }
                    }
                }
            }
        }

        self.commit_transaction(tx)
    }
}

#[derive(Debug)]
struct DbFeedKey<'a> {
    feed_url: &'a str,
}

impl<'a> DbFeedKey<'a> {
    fn from_bytes(source: &'a [u8]) -> Result<Self, Error> {
        let feed_url = std::str::from_utf8(source).map_err(|e| {
            (
                format!(
                    "Failed to decode feed key from database (source: {:?})",
                    source
                ),
                e,
            )
        })?;
        Ok(DbFeedKey { feed_url: feed_url })
    }

    fn to_bytes(&self) -> &'a [u8] {
        self.feed_url.as_bytes()
    }
}

#[derive(Debug)]
struct DbFeedValue {
    when_added: std::time::SystemTime,
}

#[derive(Debug, Deserialize, Serialize)]
struct DbFeedValueSerial {
    when_added: (u64, u32), // (seconds, sub-second nanoseconds) since epoch
}

impl DbFeedValue {
    fn to_bytes(&self) -> Vec<u8> {
        let (secs, nanos) = system_time_to_parts(self.when_added);
        rmp_serde::to_vec(&DbFeedValueSerial { when_added: (secs, nanos) }).unwrap()
    }
}

#[derive(Debug)]
struct DbItemKey<'a> {
    feed_url: &'a str,
    item_id: &'a str,
}

#[derive(Debug)]
struct DbItemSearch<'a> {
    feed_url: &'a str,
    item_id: Option<&'a str>,
}

impl<'a> DbItemKey<'a> {
    /*
    fn from_bytes(source: &'a [u8]) -> Result<Self, Error> {
        let mut split = source.splitn(2, |&c| c == 0);
        let feed_url = std::str::from_utf8(split.next().unwrap())
            .map_err(|e| {
                         (format!("Failed to decode feed item feed URL field (source: {:?})",
                                  source),
                          e)
                     })?;
        let item_id =
            std::str::from_utf8(split
                                    .next()
                                    .ok_or(format!("Feed item in database is missing field separator (source: {:?})",
                                                   source))?)
                    .map_err(|e| {
                                 (format!("Failed to decode feed item feed id field (source: {:?})",
                                          source),
                                  e)
                             })?;
        Ok(DbItemKey {
               feed_url: feed_url,
               item_id: item_id,
           })
    }
    */

    fn to_bytes(&self) -> Vec<u8> {
        DbItemSearch {
            feed_url: self.feed_url,
            item_id: Some(self.item_id),
        }.to_bytes()
    }
}

impl<'a> DbItemSearch<'a> {
    fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(self.feed_url.as_bytes());
        v.push(0);
        if let Some(item_id) = self.item_id {
            v.extend(item_id.as_bytes())
        }
        v
    }
}

#[derive(Debug)]
struct DbItemValue {
    when_first_fetched: std::time::SystemTime,
    when_last_fetched: std::time::SystemTime,
}

#[derive(Debug, Deserialize, Serialize)]
struct DbItemValueSerial {
    when_first_fetched: (u64, u32),
    when_last_fetched: (u64, u32),
}

impl DbItemValue {
    fn from_bytes(source: &[u8]) -> Result<Self, Error> {
        let x = rmp_serde::from_slice::<DbItemValueSerial>(source).map_err(
            |e| {
                Error::from((
                    format!(
                        "Failed to decode feed item from database (source: {:?})",
                        source
                    ),
                    e,
                ))
            },
        )?;
        Ok(DbItemValue {
            when_first_fetched: system_time_from_parts(x.when_first_fetched.0, x.when_first_fetched.1),
            when_last_fetched: system_time_from_parts(x.when_last_fetched.0, x.when_last_fetched.1),
        })
    }

    fn to_bytes(&self) -> Vec<u8> {
        let t1 = system_time_to_parts(self.when_first_fetched);
        let t2 = system_time_to_parts(self.when_last_fetched);
        rmp_serde::to_vec(&DbItemValueSerial {
            when_first_fetched: (t1.0, t1.1),
            when_last_fetched: (t2.0, t2.1),
        }).unwrap()
    }
}

fn system_time_to_parts(t: std::time::SystemTime) -> (u64, u32) {
    let d = t.duration_since(std::time::UNIX_EPOCH).unwrap();
    (d.as_secs(), d.subsec_nanos())
}

fn system_time_from_parts(secs: u64, subsec_nanos: u32) -> std::time::SystemTime {
    std::time::UNIX_EPOCH + std::time::Duration::new(secs, subsec_nanos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use feed::{Feed, FeedItem, FeedMeta, MockFetcher, RecorderSender};
    use tempdir::TempDir;

    const TEST_PATH_PREFIX: &str = "rss2email";

    #[test]
    fn creating_a_model_requires_it_to_not_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db_path = tdir.path().join("foo");
        Model::create(&db_path).unwrap();
        Model::create(&db_path).unwrap_err();
    }

    #[test]
    fn opening_a_model_requires_it_to_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db_path = tdir.path().join("foo");
        Model::open(&db_path).unwrap_err();
        Model::create(&db_path).unwrap();
        Model::open(&db_path).unwrap();
    }

    #[test]
    fn adding_a_feed_requires_it_to_not_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db = Model::create(&tdir.path().join("foo")).unwrap();
        db.add_feed("https://xkcd.com/rss.xml").unwrap();
        // db.add_feed("https://xkcd.com/rss.xml").unwrap_err();
    }

    #[test]
    fn removing_a_feed_requires_it_to_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db = Model::create(&tdir.path().join("foo")).unwrap();
        db.remove_feed("https://xkcd.com/rss.xml").unwrap_err();
        db.add_feed("https://xkcd.com/rss.xml").unwrap();
        db.remove_feed("https://xkcd.com/rss.xml").unwrap();
    }

    #[test]
    fn only_new_feed_items_are_sent() {

        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db = Model::create(&tdir.path().join("foo")).unwrap();
        db.add_feed("http://example.com").unwrap();
        let logger = Arc::new(Logger::new(LogLevel::Nothing));

        let fetcher = MockFetcher::from(vec![
            Ok(Feed {
                meta: FeedMeta {
                    feed_url: String::from("http://example.com"),
                    title: String::from("Example"),
                },
                items: vec![
                    FeedItem {
                        id: String::from("id alpha"),
                        title: Some(String::from("entry alpha")),
                        link: Some(String::from("http://example.com/alpha")),
                        content: Some(String::from("blah blah blah")),
                    },
                ],
            }),
        ]);

        let sender = RecorderSender::new();
        db.fetch_and_send_feeds(
            logger.clone(),
            fetcher.clone(),
            &sender,
            &FetchAndSendOptions::default(),
        ).unwrap();
        let got_items = sender.recorded_items();
        assert_eq!(
            got_items,
            &[
                (
                    FeedMeta {
                        feed_url: String::from("http://example.com"),
                        title: String::from("Example"),
                    },
                    FeedItem {
                        id: String::from("id alpha"),
                        title: Some(String::from("entry alpha")),
                        link: Some(String::from("http://example.com/alpha")),
                        content: Some(String::from("blah blah blah")),
                    },
                ),
            ]
        );

        let sender = RecorderSender::new();
        db.fetch_and_send_feeds(
            logger.clone(),
            fetcher.clone(),
            &sender,
            &FetchAndSendOptions::default(),
        ).unwrap();
        let got_items = sender.recorded_items();
        assert_eq!(got_items, &[]);
    }
}
