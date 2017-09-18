extern crate atom_syndication;
extern crate byteorder;
extern crate chrono;
extern crate clap;
extern crate escapade;
extern crate futures;
extern crate lettre;
extern crate reqwest;
extern crate rmp_serde;
extern crate rss;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[cfg(test)]
extern crate tempdir;
extern crate toml;

mod config;
mod error;
mod log;
mod model;

pub use error::Error;

use std::io::Write;
use std::sync::Arc;

const CONFIG_PATH: &str = "rss2email.conf";
const DB_PATH: &str = "rss2email.db";

struct FakeDebug<T>(T);

impl<T> std::fmt::Debug for FakeDebug<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        "(no debug info available)".fmt(f)
    }
}

impl<T> std::ops::Deref for FakeDebug<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> std::ops::DerefMut for FakeDebug<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

fn main_impl() -> Result<(), Error> {

    use clap::{App, Arg, SubCommand};
    use model::Database;

    let mut app = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .subcommand(
            SubCommand::with_name("add")
                .about("Add feed to the database")
                .arg(
                    Arg::with_name("FEED_URL")
                        .help("URL of the feed to add")
                        .required(true),
                ),
        )
        .subcommand(SubCommand::with_name("create").about("Create database"))
        .subcommand(
            SubCommand::with_name("fetch")
                .about("Fetch feeds and send emails for new items")
                .arg(
                    Arg::with_name("VERBOSE")
                        .short("v")
                        .long("verbose")
                        .multiple(true)
                        .help("Print more information"),
                )
                .arg(Arg::with_name("NO_SEND").long("no-send").help(
                    "Run as normal but do not send emails",
                ))
                .arg(Arg::with_name("FEED_URL").multiple(true).help(
                    "URL of the feed(s) to fetch",
                )),
        )
        .subcommand(SubCommand::with_name("list").about("Print all feed URLs"))
        .subcommand(
            SubCommand::with_name("remove")
                .about("Remove a feed from the database")
                .arg(
                    Arg::with_name("FEED_URL")
                        .help("URL of the feed to remove")
                        .required(true),
                ),
        );

    let matches = app.clone().get_matches();

    if let Some(matches) = matches.subcommand_matches("add") {
        let feed_url = matches.value_of("FEED_URL").unwrap();
        let mut db = Database::open(DB_PATH)?;
        db.add_feed(feed_url)?;
        db.commit()?;
    } else if let Some(_matches) = matches.subcommand_matches("create") {
        let db = Database::create(DB_PATH)?;
        db.commit()?;
    } else if let Some(matches) = matches.subcommand_matches("fetch") {
        let config = config::Config::load(CONFIG_PATH)?;
        let mut db = Database::open(DB_PATH)?;
        let log_level = match matches.occurrences_of("VERBOSE") {
            0 => log::LogLevel::Normal,
            _ => log::LogLevel::Verbose,
        };
        let logger = Arc::new(log::Logger::new(log_level));
        let fetcher = model::NetFetcher::new()?;
        let sender = model::EmailSender::new(&config)?;
        let mut options = model::FetchAndSendOptions::new();
        options.with_no_send(matches.is_present("NO_SEND"));
        if let Some(feed_urls) = matches.values_of("FEED_URL") {
            options.with_feed_urls(feed_urls);
        }
        db.fetch_and_send_feeds(logger, fetcher, &sender, &options)?;
        db.commit()?;
    } else if let Some(_matches) = matches.subcommand_matches("list") {
        let db = Database::open(DB_PATH)?;
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        for feed_url in db.feed_urls() {
            writeln!(w, "{}", feed_url).unwrap();
        }
    } else if let Some(matches) = matches.subcommand_matches("remove") {
        let feed_url = matches.value_of("FEED_URL").unwrap();
        let mut db = Database::open(DB_PATH)?;
        db.remove_feed(feed_url)?;
        db.commit()?;
    } else {
        app.print_help().unwrap();
        println!(); // print_help omits final newline
    }

    Ok(())
}

fn main() {
    match main_impl() {
        Ok(..) => {}
        Err(e) => {
            let stderr = std::io::stderr();
            const UNKNOWN_EXE: &str = "???";
            let name = std::env::current_exe()
                .map(|x| {
                    x.file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or(String::from(UNKNOWN_EXE))
                })
                .unwrap_or(String::from(UNKNOWN_EXE));
            writeln!(stderr.lock(), "{}: *** {}", name, e).unwrap();
            std::process::exit(1);
        }
    }
}
