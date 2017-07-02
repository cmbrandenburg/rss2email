extern crate atom_syndication;
extern crate byteorder;
extern crate clap;
extern crate escapade;
extern crate futures;
extern crate lettre;
extern crate lmdb;
extern crate lmdb_sys;
extern crate reqwest;
extern crate rmp_serde;
extern crate rss;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[cfg(test)]
extern crate tempdir;
extern crate toml;

mod config;
mod error;
mod feed;
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
    use model::Model;

    let matches = App::new(env!("CARGO_PKG_NAME"))
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
        .subcommand(SubCommand::with_name("list").about("Print all feed URLs"))
        .subcommand(
            SubCommand::with_name("remove")
                .about("Remove a feed from the database")
                .arg(
                    Arg::with_name("FEED_URL")
                        .help("URL of the feed to remove")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("run")
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
                    "URL of the feed(s) to run",
                )),
        )
        .get_matches();

    if let Some(matches) = matches.subcommand_matches("add") {
        let feed_url = matches.value_of("FEED_URL").unwrap();
        let model = Model::open(DB_PATH)?;
        model.add_feed(feed_url)?;
    } else if let Some(_matches) = matches.subcommand_matches("create") {
        Model::create(DB_PATH)?;
    } else if let Some(_matches) = matches.subcommand_matches("list") {
        let model = Model::open(DB_PATH)?;
        let stdout = std::io::stdout();
        let mut w = stdout.lock();
        model.for_each_feed(|feed_url| {
            writeln!(w, "{}", feed_url).unwrap();
            Ok(())
        })?;
    } else if let Some(matches) = matches.subcommand_matches("remove") {
        let feed_url = matches.value_of("FEED_URL").unwrap();
        let model = Model::open(DB_PATH)?;
        model.remove_feed(feed_url)?;
    } else if let Some(matches) = matches.subcommand_matches("run") {
        let config = config::Config::load(CONFIG_PATH)?;
        let model = Model::open(DB_PATH)?;
        let log_level = match matches.occurrences_of("VERBOSE") {
            0 => log::LogLevel::Normal,
            _ => log::LogLevel::Verbose,
        };
        let logger = Arc::new(log::Logger::new(log_level));
        let fetcher = feed::NetFetcher::new()?;
        let sender = feed::EmailSender::new(&config)?.with_no_send(
            matches.is_present(
                "NO_SEND",
            ),
        );
        let options = model::FetchAndSendOptions {
            feed_urls: matches.values_of("FEED_URL").map(|x| {
                x.map(|x| String::from(x)).collect()
            }),
        };
        model.fetch_and_send_feeds(
            logger,
            fetcher,
            &sender,
            &options,
        )?;
    } else {
        unreachable!();
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
