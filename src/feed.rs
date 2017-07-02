use {Error, FakeDebug, atom_syndication, futures, lettre, reqwest, rss, std};
use config::Config;
use escapade::Escapable;
use log::{LogKind, LogLevel, Logger};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const NUM_FETCHERS: usize = 32;
const CHANNEL_CAPACITY: usize = 2 * NUM_FETCHERS;
const FETCH_TIMEOUT_SECS: u64 = 60;

#[derive(Clone, Debug, PartialEq)]
pub struct Feed {
    pub meta: FeedMeta,
    pub items: Vec<FeedItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedMeta {
    pub feed_url: String, // this is the same URL as what's in the database
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedItem {
    pub id: String,
    pub title: Option<String>,
    pub link: Option<String>,
    pub content: Option<String>,
}

pub trait Sender {
    fn send(&self, logger: &Logger, feed_meta: &FeedMeta, feed_item: &FeedItem) -> Result<(), Error>;
}

#[cfg(test)]
#[derive(Debug)]
pub struct RecorderSender {
    recorded_items: Mutex<Vec<(FeedMeta, FeedItem)>>,
}

#[cfg(test)]
impl RecorderSender {
    pub fn new() -> Self {
        RecorderSender { recorded_items: Mutex::new(Vec::new()) }
    }

    pub fn recorded_items(self) -> Vec<(FeedMeta, FeedItem)> {
        self.recorded_items.into_inner().unwrap()
    }
}

#[cfg(test)]
impl Sender for RecorderSender {
    fn send(&self, _logger: &Logger, feed_meta: &FeedMeta, feed_item: &FeedItem) -> Result<(), Error> {
        self.recorded_items.lock().unwrap().push((
            feed_meta.clone(),
            feed_item.clone(),
        ));
        Ok(())
    }
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
            .map_err(|e| ("Failed to construct mail client", e))?
            .credentials(&config.smtp_username, &config.smtp_password)
            .security_level(lettre::transport::smtp::SecurityLevel::AlwaysEncrypt)
            .build();

        Ok(EmailSender {
            config: config.clone(),
            mail_client: FakeDebug(Mutex::new(mail_client)),
            no_send: false,
        })
    }

    pub fn with_no_send(mut self, no_send: bool) -> Self {
        self.no_send = no_send;
        self
    }
}

impl Sender for EmailSender {
    fn send(&self, logger: &Logger, feed_meta: &FeedMeta, feed_item: &FeedItem) -> Result<(), Error> {

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
                feed_meta.title.as_ref(),
            ))
            .subject(&item_title)
            .header(("Content-Type", "text/html"))
            .body(&body)
            .build()
            .map_err(|e| ("Failed to construct email message", e))?;

        /*
        use std::io::Write;
        let stdout = std::io::stdout();
        writeln!(stdout.lock(), "Sending {:?}", email).unwrap();
        */

        if !self.no_send {

            logger.log(
                LogLevel::Verbose,
                LogKind::Info,
                format!("Sending {} — {:?}", feed_meta.feed_url, item_title),
            );

            self.mail_client.lock().unwrap().send(email).map_err(|e| {
                (
                    format!(
                        "Failed to send email (feed url: {}, feed item id: {})",
                        feed_meta.feed_url,
                        feed_item.id
                    ),
                    e,
                )
            })?;
        } else {
            logger.log(
                LogLevel::Verbose,
                LogKind::Info,
                format!("Not sending {} — {:?}", feed_meta.feed_url, item_title),
            );
        }

        Ok(())
    }
}

pub trait Fetcher {
    type Stream: futures::Stream<Item = Feed, Error = Error>;
    fn fetch(self, logger: Arc<Logger>, feed_urls: Vec<String>) -> Self::Stream;
}

#[derive(Debug)]
pub struct NetFetcher {
    client: Arc<Mutex<reqwest::Client>>,
}

impl NetFetcher {
    pub fn new() -> Result<Self, Error> {

        let mut client = reqwest::Client::new().map_err(|e| {
            ("Failed to construct HTTP client", e)
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
        send_chan: Arc<Mutex<futures::sink::Wait<futures::sync::mpsc::Sender<Result<Feed, String>>>>>,
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
                (format!("Failed to fetch feed (feed URL: {}): {}", feed_url, e))
            })?;

            let mut body = String::new();
            response.read_to_string(&mut body).map_err(|e| {
                (format!("Failed to read feed body (feed URL: {}): {}", feed_url, e))
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

                    match send_chan.lock().unwrap().send(Ok(feed)) {
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
    recv_chan: futures::sync::mpsc::Receiver<Result<Feed, String>>,
}

impl futures::Stream for NetFetcherStream {
    type Item = Feed;
    type Error = Error;
    fn poll(&mut self) -> futures::Poll<Option<Self::Item>, Self::Error> {
        match self.recv_chan.poll().unwrap() {
            futures::Async::NotReady => Ok(futures::Async::NotReady),
            futures::Async::Ready(None) => Ok(futures::Async::Ready(None)),
            futures::Async::Ready(Some(Err(e))) => Err(Error::from(e)),
            futures::Async::Ready(Some(Ok(x))) => Ok(futures::Async::Ready(Some(x))),
        }
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct MockFetcher {
    mock_items: Vec<Result<Feed, String>>,
}

#[cfg(test)]
impl Fetcher for MockFetcher {
    type Stream = futures::stream::Iter<std::vec::IntoIter<Result<Feed, Error>>>;
    fn fetch(self, _logger: Arc<Logger>, _feed_urls: Vec<String>) -> Self::Stream {

        let items = self.mock_items
            .into_iter()
            .map(|x| match x {
                Err(s) => Err(Error::from(s)),
                Ok(x) => Ok(x),
            })
            .collect::<Vec<_>>();

        futures::stream::iter(items)
    }
}

#[cfg(test)]
impl From<Vec<Result<Feed, String>>> for MockFetcher {
    fn from(mock_items: Vec<Result<Feed, String>>) -> Self {
        MockFetcher { mock_items: mock_items }
    }
}

fn parse_syndication(feed_url: &str, body: &str) -> Result<Feed, Error> {

    // First try as RSS, then as Atom.

    match rss::Channel::read_from(std::io::Cursor::new(body)) {
        Err(..) => {}
        Ok(channel) => {

            return Ok(Feed {
                meta: FeedMeta {
                    feed_url: String::from(feed_url),
                    title: String::from(channel.title()),
                },
                items: channel
                    .items()
                    .iter()
                    .map(|item| -> Result<FeedItem, Error> {
                        Ok(FeedItem {
                            id: item.guid()
                                .map(|x| String::from(x.value()))
                                .or(item.link().map(|x| String::from(x)))
                                .ok_or(format!(
                                    "Cannot determine unique identifier for RSS item (feed URL: {})",
                                    feed_url
                                ))?,
                            title: item.title().map(|x| String::from(x)),
                            link: item.link().map(|x| String::from(x)),
                            content: item.content().or(item.description()).map(
                                |x| String::from(x),
                            ),
                        })
                    })
                    .collect::<Result<_, _>>()?,
            });
        }
    }

    let raw = atom_syndication::Feed::from_str(body).map_err(|e| {
        ((format!("Failed to parse feed (feed URL: {})", feed_url), e))
    })?;

    Ok(Feed {
        meta: FeedMeta {
            feed_url: String::from(feed_url),
            title: raw.title,
        },
        items: raw.entries
            .into_iter()
            .map(|entry| {
                FeedItem {
                    id: entry.id,
                    title: Some(entry.title),
                    link: entry.links.first().map(|x| x.href.clone()),
                    content: entry.content.map(|x| match x {
                        atom_syndication::Content::Text(x) => x.escape().into_inner(),
                        atom_syndication::Content::Html(x) => x,
                        atom_syndication::Content::Xhtml(x) => x.to_string(),
                    }),
                }
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            meta: FeedMeta {
                feed_url: String::from("http://example.com"),
                title: String::from("alpha"),
            },
            items: vec![
                FeedItem {
                    id: String::from("golf"),
                    title: Some(String::from("delta")),
                    link: Some(String::from("http://echo")),
                    content: Some(String::from("foxtrot")),
                },
            ],
        };

        assert_eq!(got, expected);

    }

}
