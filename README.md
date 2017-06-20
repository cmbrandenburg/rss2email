# rss2email

Command line tool to send RSS/Atom items via email.

## What is this?

This project is a rewrite of the [Python program of the same
name](http://www.allthingsrss.com/rss2email/). The primary goal of this
rewrite is to improve performance, which has been accomplished.
Typically, it fetches and processes dozens of feeds within a few
seconds—though, this depends on the speed of the outgoing SMTP server.

## How to build this?

To build this project, you must have [Rust](https://rustup.rs/)
installed.

```
$ git clone https://github.com/cmbrandenburg/rss2email.git &&
  cd rss2email &&
  cargo build --release &&
  cargo install
```

Alternatively, you can copy the newly built executable into a directory
of your choice.

```
$ install target/release/rss2email ~/bin/
```

## How to run this?

To run the `rss2email` program, you must create a configuration file
called `rss2email.conf` within the current working directory. Currently,
the configuration file contains four settings, all mandatory.

```
$ cat rss2email.conf
smtp_server = "smtp.gmail.com:587"
smtp_username = "your-email-address-to-send-from@gmail.com"
smtp_password = "your-password-for-the-from-address"
recipient = "email-address-to-send-to@example.com"
```

Now, from within the same directory as the `rss2email.conf`
configuration file, first create a database, then add feeds, then lastly
run to fetch-and-send those feeds.

```
$ rss2email create &&
  rss2email add https://xkcd.com/rss.xml
  rss2email run -v
Fetching https://xkcd.com/rss.xml
Sending https://xkcd.com/rss.xml — "Refresh Types"
Sending https://xkcd.com/rss.xml — "Once Per Day"
Sending https://xkcd.com/rss.xml — "Election Map"
Sending https://xkcd.com/rss.xml — "Magnetohydrodynamics"
```

The `rss2email` remembers feed items it has fetched and sent, and it
will skip re-sending those items in later runs. This makes `rss2email`
ideal for running as a cron job, whereby it periodically runs and sends
only new feed items.

For more information about `rss2email`, please run `rss2email help`.

## Contact

If you have questions or comments, you may email me at
[c.m.brandenburg@gmail.com](mailto:c.m.brandenburg@gmail.com).

Pull requests are welcome.
