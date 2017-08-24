use {Error, FakeDebug, atom_syndication, futures, lettre, reqwest, rss, serde_json, std};
use chrono::{DateTime, Utc};
use config::Config;
use escapade::Escapable;
use log::{LogKind, LogLevel, Logger};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const NUM_FETCHERS: usize = 32;
const CHANNEL_CAPACITY: usize = 2 * NUM_FETCHERS;
const FETCH_TIMEOUT_SECS: u64 = 60;

#[derive(Debug)]
pub struct Database {
    path: PathBuf,
    feeds: HashMap<String, Feed>,
}

impl Database {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self, Error> {

        let path = path.as_ref();

        match std::fs::metadata(path) {
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(
                Error::new(format!("Failed to obtain file metadata (path: {:?})", path))
                    .with_cause(e)
                    .into_error(),
            ),
            Ok(..) => return Err(
                Error::new(format!("Database already exists (path: {:?})", path)).into_error(),
            ),
        }

        Ok(Database {
            path: PathBuf::from(path),
            feeds: HashMap::new(),
        })
    }

    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, Error> {

        let path = path.as_ref();

        let f = std::fs::File::open(path).map_err(|e| {
            Error::new(format!("Failed to open database (path: {:?})", path))
                .with_cause(e)
                .into_error()
        })?;

        let feeds = serde_json::from_reader(f).map_err(|e| {
            Error::new(format!("Database is corrupt (path: {:?})", path))
                .with_cause(e)
                .into_error()
        })?;

        Ok(Database {
            path: PathBuf::from(path),
            feeds,
        })
    }

    pub fn commit(&self) -> Result<(), Error> {

        // This employs the write-sync-rename pattern to guarantee an atomic
        // update.

        let working_path = {
            let mut p = self.path.clone();
            p.set_extension("working");
            p
        };

        let mut f = std::fs::File::create(&working_path).map_err(|e| {
            Error::new(format!(
                "Failed to create replacement database (path: {:?})",
                working_path
            )).with_cause(e)
                .into_error()
        })?;

        serde_json::to_writer(f.by_ref(), &self.feeds).map_err(
            |e| {
                Error::new(format!(
                    "Failed to write feeds to database (path: {:?})",
                    working_path
                )).with_cause(e)
                    .into_error()
            },
        )?;

        f.write_all(b"\n").map_err(|e| {
            Error::new(format!(
                "Failed to write trailing newline to database (path: {:?})",
                working_path
            )).with_cause(e)
                .into_error()
        })?;

        f.sync_all().map_err(|e| {
            Error::new(format!(
                "Failed to sync database to disk (path: {:?})",
                working_path
            )).with_cause(e)
                .into_error()
        })?;

        std::fs::rename(&working_path, &self.path).map_err(|e| {
            Error::new(format!(
                "Failed to replace database (destination: {:?}, source: {:?})",
                self.path,
                working_path
            )).with_cause(e)
                .into_error()
        })?;

        Ok(())
    }

    pub fn add_feed(&mut self, feed_url: &str) -> Result<(), Error> {

        if self.feeds.contains_key(feed_url) {
            return Err(
                Error::new(format!(
                    "Feed already exists in database (feed URL: {:?})",
                    feed_url
                )).into_error(),
            );
        }

        self.feeds.insert(String::from(feed_url), Feed::new());

        Ok(())
    }

    pub fn remove_feed(&mut self, feed_url: &str) -> Result<(), Error> {
        match self.feeds.remove(feed_url) {
            None => Err(
                Error::new(format!(
                    "Feed does not exist in database (feed URL: {:?})",
                    feed_url
                )).into_error(),
            ),
            Some(_) => Ok(()),
        }
    }

    pub fn feed_urls<'a>(&'a self) -> Box<Iterator<Item = &'a str> + 'a> {
        Box::new(self.feeds.iter().map(|(k, _)| k.as_str()))
    }

    pub fn fetch_and_send_feeds<F, S>(
        &mut self,
        logger: Arc<Logger>,
        fetcher: F,
        sender: &S,
        options: &FetchAndSendOptions,
    ) -> Result<(), Error>
    where
        F: Fetcher,
        S: Sender,
    {
        fn item_ids(feed: &Feed) -> HashSet<String> {
            feed.items.keys().map(|x| x.clone()).collect()
        }

        // Pump the fetcher for the feeds it receives. Send each *new* item we
        // receive. Update database state as we go.
        //
        // In case of error, stop all processing. Make sure that the database
        // reflects all sent items but no unsent items.

        let feeds_to_fetch = self.feeds
            .keys()
            .filter(|x| options.should_fetch(x))
            .map(|x| x.clone())
            .collect::<Vec<_>>();

        let mut spawn = futures::executor::spawn(fetcher.fetch(logger.clone(), feeds_to_fetch));
        'outer: while let Some(fetch_result) = spawn.wait_stream() {

            let (feed_url, mut new_feed) = match fetch_result {
                Err(e) => {
                    logger.log(LogLevel::Important, LogKind::Error, e);
                    break;
                }
                Ok(x) => x,
            };

            let new_item_ids = item_ids(&new_feed);

            let old_feed = self.feeds.get_mut(&feed_url).unwrap();
            let old_item_ids = item_ids(&old_feed);

            if old_feed.title != new_feed.title && new_feed.title.is_some() {
                old_feed.title = new_feed.title.clone();
            }

            for item_id in new_item_ids.difference(&old_item_ids).collect::<Vec<_>>() {

                let item = new_feed.items.remove(item_id).unwrap();

                logger.log(
                    LogLevel::Verbose,
                    LogKind::Info,
                    format!(
                        "{} {} — {:?}",
                        if options.no_send { "Not sending" } else { "Sending" },
                        feed_url,
                        item.title.as_ref().map(|x| x.as_str()).unwrap_or("n/a")
                    ),
                );

                if !options.no_send {
                    if let Err(e) = sender.send(&feed_url, &new_feed, &item_id, &item) {
                        logger.log(
                            LogLevel::Important,
                            LogKind::Error,
                            format!(
                                "An error occurred while sending (feed id = {}): {}",
                                item_id,
                                e
                            ),
                        );
                        break 'outer; // stop all processing
                    }
                }

                old_feed.items.insert(item_id.clone(), item);
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Feed {
    title: Option<String>,
    items: HashMap<String, FeedItem>, // id to item
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FeedItem {
    last_observed: DateTime<Utc>,
    #[serde(skip_serializing)]
    title: Option<String>,
    #[serde(skip_serializing)]
    link: Option<String>,
    #[serde(skip_serializing)]
    content: Option<String>,
}

impl Feed {
    fn new() -> Self {
        Feed {
            title: None,
            items: HashMap::new(),
        }
    }
}

pub trait Sender {
    fn send(&self, feed_url: &str, feed: &Feed, feed_item_id: &str, feed_item: &FeedItem) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct EmailSender {
    config: Config,
    mail_client: FakeDebug<Mutex<lettre::transport::smtp::SmtpTransport>>, // TODO: Need newer lettre crate for Debug impl
    no_send: bool,
}

impl EmailSender {
    pub fn new(config: &Config) -> Result<Self, Error> {

        let mail_client = lettre::transport::smtp::SmtpTransportBuilder::new(&config.smtp_server)
            .map_err(|e| {
                Error::new("Failed to construct mail client")
                    .with_cause(e)
                    .into_error()
            })?
            .credentials(&config.smtp_username, &config.smtp_password)
            .security_level(lettre::transport::smtp::SecurityLevel::AlwaysEncrypt)
            .build();

        Ok(EmailSender {
            config: config.clone(),
            mail_client: FakeDebug(Mutex::new(mail_client)),
            no_send: false,
        })
    }
}

impl Sender for EmailSender {
    fn send(&self, feed_url: &str, feed: &Feed, feed_item_id: &str, feed_item: &FeedItem) -> Result<(), Error> {

        use lettre::transport::EmailTransport;

        let item_title = feed_item.title.as_ref().map(|x| x.as_str()).unwrap_or(
            "(N/a)",
        );

        let item_content = feed_item.content.as_ref().map(|x| x.as_str()).unwrap_or("");

        let body = match feed_item.link {
            None => format!(
                r#"<h1>{}</h1>{}"#,
                item_title.escape().into_inner(),
                item_content
            ),
            Some(ref link) => format!(
                r#"<h1><a href="{}">{}</a></h1>{}<p><a href="{}">{}</a></p>"#,
                link.escape().into_inner(),
                item_title.escape().into_inner(),
                item_content,
                link.escape().into_inner(),
                link.escape().into_inner()
            ),
        };

        let email = lettre::email::EmailBuilder::new()
            .to(self.config.recipient.as_ref())
            .from((
                self.config.smtp_username.as_ref(),
                feed.title.as_ref().map(|x| x.as_ref()).unwrap(),
            ))
            .subject(&item_title)
            .header(("Content-Type", "text/html"))
            .body(&body)
            .build()
            .map_err(|e| {
                Error::new("Failed to construct email message")
                    .with_cause(e)
                    .into_error()
            })?;

        /*
        use std::io::Write;
        let stdout = std::io::stdout();
        writeln!(stdout.lock(), "Sending {:?}", email).unwrap();
        */

        if !self.no_send {
            self.mail_client.lock().unwrap().send(email).map_err(|e| {
                Error::new(format!(
                    "Failed to send email (feed url: {}, feed item id: {})",
                    feed_url,
                    feed_item_id
                )).with_cause(e)
                    .into_error()
            })?;
        }

        Ok(())
    }
}

pub trait Fetcher {
    type Stream: futures::Stream<Item = (String, Feed), Error = Error>;
    fn fetch(self, logger: Arc<Logger>, feed_urls: Vec<String>) -> Self::Stream;
}

#[derive(Debug, Default)]
pub struct FetchAndSendOptions {
    feed_urls: Option<HashSet<String>>,
    no_send: bool,
}

impl FetchAndSendOptions {
    pub fn new() -> Self {
        FetchAndSendOptions {
            feed_urls: None,
            no_send: false,
        }
    }

    pub fn with_feed_urls<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, feed_urls: I) -> &mut Self {
        self.feed_urls = Some(feed_urls.into_iter().map(|x| x.into()).collect());
        self
    }

    pub fn with_no_send(&mut self, no_send: bool) -> &mut Self {
        self.no_send = no_send;
        self
    }

    fn should_fetch(&self, feed_url: &str) -> bool {
        if let Some(ref m) = self.feed_urls {
            return m.contains(feed_url);
        }
        true
    }
}

#[derive(Debug)]
pub struct NetFetcher {
    client: Arc<Mutex<reqwest::Client>>,
}

impl NetFetcher {
    pub fn new() -> Result<Self, Error> {

        let mut client = reqwest::Client::new().map_err(|e| {
            Error::new("Failed to construct HTTP client")
                .with_cause(e)
                .into_error()
        })?;

        client.timeout(std::time::Duration::new(FETCH_TIMEOUT_SECS, 0));

        Ok(NetFetcher { client: Arc::new(Mutex::new(client)) })
    }

    // It's kinda poor to wrap a channel in an Arc<Mutex<>>, but we need the
    // Sync and Send traits.
    fn fetch_thread(
        logger: Arc<Logger>,
        client: Arc<Mutex<reqwest::Client>>,
        feed_urls: Arc<Mutex<Vec<String>>>,
        send_chan: Arc<Mutex<futures::sink::Wait<futures::sync::mpsc::Sender<Result<(String, Feed), String>>>>>,
    ) {

        let fetch_it = |feed_url: &str| -> Result<String, Error> {

            use std::io::Read;

            logger.log(
                LogLevel::Normal,
                LogKind::Info,
                format!("Fetching {}", feed_url),
            );

            let request = {
                client.lock().unwrap().get(feed_url)
            };

            let mut response = request.send().map_err(|e| {
                Error::new(format!("Failed to fetch feed (feed URL: {})", feed_url))
                    .with_cause(e)
                    .into_error()
            })?;

            let mut body = String::new();
            response.read_to_string(&mut body).map_err(|e| {
                Error::new(format!("Failed to read feed body (feed URL: {})", feed_url))
                    .with_cause(e)
                    .into_error()
            })?;

            Ok(body)
        };

        loop {
            let feed_url = match feed_urls.lock().unwrap().pop() {
                None => return, // no more feeds to fetch
                Some(x) => x,
            };

            // As soon as the channel closes, exit this thread.

            match fetch_it(&feed_url) {
                Err(e) => {
                    match send_chan.lock().unwrap().send(Err(e.to_string())) {
                        Err(_) => return, // channel closed
                        Ok(_) => {}
                    }
                }
                Ok(body) => {

                    let feed = match parse_syndication(&feed_url, &body) {
                        Err(e) => {
                            logger.log(LogLevel::Important, LogKind::Error, e);
                            continue;
                        }
                        Ok(x) => x,
                    };

                    match send_chan.lock().unwrap().send(Ok((feed_url, feed))) {
                        Err(_) => return, // channel closed
                        Ok(_) => {}
                    }
                }
            }
        }
    }
}

impl Fetcher for NetFetcher {
    type Stream = NetFetcherStream;
    fn fetch(self, logger: Arc<Logger>, feed_urls: Vec<String>) -> Self::Stream {

        // We use (possibly) multiple fetcher threads, each of which sends
        // the feeds it receives through a channel to the stream poller.

        let feed_urls = Arc::new(Mutex::new(feed_urls));
        let (send_chan, recv_chan) = futures::sync::mpsc::channel(CHANNEL_CAPACITY);

        let threads = (0..NUM_FETCHERS)
            .into_iter()
            .map(|_| {
                let logger = logger.clone();
                let client = self.client.clone();
                let feed_urls = feed_urls.clone();
                let send_chan = send_chan.clone();
                std::thread::spawn(move || {
                    use futures::Sink;
                    Self::fetch_thread(
                        logger,
                        client,
                        feed_urls,
                        Arc::new(Mutex::new(send_chan.wait())),
                    )
                })
            })
            .collect();

        NetFetcherStream {
            threads: threads,
            recv_chan: recv_chan,
        }
    }
}

#[derive(Debug)]
pub struct NetFetcherStream {
    threads: Vec<std::thread::JoinHandle<()>>,
    recv_chan: futures::sync::mpsc::Receiver<Result<(String, Feed), String>>,
}

impl futures::Stream for NetFetcherStream {
    type Item = (String, Feed);
    type Error = Error;
    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        match self.recv_chan.poll().unwrap() {
            futures::Async::NotReady => Ok(futures::Async::NotReady),
            futures::Async::Ready(None) => Ok(futures::Async::Ready(None)),
            futures::Async::Ready(Some(Err(e))) => Err(Error::new(e).into_error()),
            futures::Async::Ready(Some(Ok(x))) => Ok(futures::Async::Ready(Some(x))),
        }
    }
}

fn parse_syndication(feed_url: &str, body: &str) -> Result<Feed, Error> {

    // First try as RSS, then as Atom.

    match rss::Channel::read_from(std::io::Cursor::new(body)) {
        Err(..) => {}
        Ok(channel) => {
            return Ok(Feed {
                title: Some(String::from(channel.title())),
                items: channel
                    .items()
                    .iter()
                    .map(|item| -> Result<(String, FeedItem), Error> {
                        let id = item.guid()
                            .map(|x| String::from(x.value()))
                            .or(item.link().map(|x| String::from(x)))
                            .ok_or(
                                Error::new(format!(
                                    "Cannot determine unique identifier for RSS item (feed URL: {})",
                                    feed_url
                                )).into_error(),
                            )?;
                        Ok((
                            id,
                            FeedItem {
                                last_observed: DateTime::from(SystemTime::now()),
                                title: item.title().map(|x| String::from(x)),
                                link: item.link().map(|x| String::from(x)),
                                content: item.content().or(item.description()).map(
                                    |x| String::from(x),
                                ),
                            },
                        ))
                    })
                    .collect::<Result<_, _>>()?,
            });
        }
    }

    let raw = atom_syndication::Feed::from_str(body).map_err(|e| {
        Error::new(format!("Failed to parse feed (feed URL: {})", feed_url))
            .with_cause(e)
            .into_error()
    })?;

    Ok(Feed {
        title: Some(String::from(raw.title())),
        items: raw.entries()
            .iter()
            .map(|entry| {
                (
                    String::from(entry.id()),
                    FeedItem {
                        last_observed: DateTime::from(SystemTime::now()),
                        title: Some(String::from(entry.title())),
                        link: entry.links().first().map(|x| String::from(x.href())),
                        content: entry.content().and_then(|x| x.value()).map(
                            |x| String::from(x),
                        ),
                    },
                )
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    const TEST_PATH_PREFIX: &str = "rss2email";

    #[derive(Debug)]
    pub struct RecorderSender {
        recorded_items: Mutex<Vec<(String, String)>>,
    }

    impl RecorderSender {
        pub fn new() -> Self {
            RecorderSender { recorded_items: Mutex::new(Vec::new()) }
        }

        pub fn recorded_items(self) -> Vec<(String, String)> {
            self.recorded_items.into_inner().unwrap()
        }
    }

    impl Sender for RecorderSender {
        fn send(&self, feed_url: &str, _feed: &Feed, feed_item_id: &str, _feed_item: &FeedItem) -> Result<(), Error> {
            self.recorded_items.lock().unwrap().push((
                String::from(feed_url),
                String::from(
                    feed_item_id,
                ),
            ));
            Ok(())
        }
    }

    #[derive(Clone, Debug)]
    pub struct MockFetcher {
        mock_items: Vec<Result<(String, Feed), String>>,
    }

    impl Fetcher for MockFetcher {
        type Stream = futures::stream::Iter<std::vec::IntoIter<Result<(String, Feed), Error>>>;
        fn fetch(self, _logger: Arc<Logger>, _feed_urls: Vec<String>) -> Self::Stream {

            let items = self.mock_items
                .into_iter()
                .map(|x| match x {
                    Err(s) => Err(Error::new(s).into_error()),
                    Ok(x) => Ok(x),
                })
                .collect::<Vec<_>>();

            futures::stream::iter(items)
        }
    }

    impl From<Vec<Result<(String, Feed), String>>> for MockFetcher {
        fn from(mock_items: Vec<Result<(String, Feed), String>>) -> Self {
            MockFetcher { mock_items: mock_items }
        }
    }

    #[test]
    fn rss_content_is_in_description() {

        let source = r#"<rss version="2.0">
<channel>
<title>alpha</title>
<link>http://bravo</link>
<description>charlie</description>
<item>
<title>delta</title>
<link>http://echo</link>
<description>foxtrot</description>
<guid>golf</guid>
</item>
</channel>
</rss>"#;

        let got = super::parse_syndication("http://example.com", source).unwrap();

        let expected = Feed {
            title: Some(String::from("alpha")),
            items: vec![
                (
                    String::from("golf"),
                    FeedItem {
                        last_observed: got.items.get("golf").unwrap().last_observed,
                        title: Some(String::from("delta")),
                        link: Some(String::from("http://echo")),
                        content: Some(String::from("foxtrot")),
                    }
                ),
            ].into_iter()
                .collect(),
        };

        assert_eq!(got, expected);
    }

    #[test]
    fn creating_a_database_requires_it_to_not_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db_path = tdir.path().join("foo");
        Database::create(&db_path).unwrap().commit().unwrap();
        Database::create(&db_path).unwrap_err();
    }

    #[test]
    fn opening_a_database_requires_it_to_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let db_path = tdir.path().join("foo");
        Database::open(&db_path).unwrap_err();
        Database::create(&db_path).unwrap().commit().unwrap();
        Database::open(&db_path).unwrap();
    }

    #[test]
    fn adding_a_feed_requires_it_to_not_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let mut db = Database::create(&tdir.path().join("foo")).unwrap();
        db.add_feed("https://xkcd.com/rss.xml").unwrap();
    }

    #[test]
    fn removing_a_feed_requires_it_to_exist() {
        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let mut db = Database::create(&tdir.path().join("foo")).unwrap();
        db.remove_feed("https://xkcd.com/rss.xml").unwrap_err();
        db.add_feed("https://xkcd.com/rss.xml").unwrap();
        db.remove_feed("https://xkcd.com/rss.xml").unwrap();
    }

    #[test]
    fn only_new_feed_items_are_sent() {

        let tdir = TempDir::new(TEST_PATH_PREFIX).unwrap();
        let mut db = Database::create(&tdir.path().join("foo")).unwrap();
        db.add_feed("http://example.com").unwrap();
        let logger = Arc::new(Logger::new(LogLevel::Nothing));

        let fetcher = MockFetcher::from(vec![
            Ok((
                String::from("http://example.com"),
                Feed {
                    title: Some(String::from("Example")),
                    items: vec![
                        (
                            String::from("id alpha"),
                            FeedItem {
                                last_observed: DateTime::from(SystemTime::now()),
                                title: Some(String::from("entry alpha")),
                                link: Some(String::from("http://example.com/alpha")),
                                content: Some(String::from("blah blah blah")),
                            }
                        ),
                    ].into_iter()
                        .collect(),
                },
            )),
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
                (String::from("http://example.com"), String::from("id alpha")),
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
