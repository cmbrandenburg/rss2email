# rss2email

Command line tool to send RSS to email

## What is this?

This project is a rewrite of the useful [Python
program of the same name](http://www.allthingsrss.com/rss2email/). That
program, which appears to have been abandoned, gets the job done but
runs excessively slow. It processes RSS feeds one at a time and takes
several minutes to handle a few dozen RSS feeds.

This project aims to capture the utility of the Python program while
running much faster. It will also compile to native code so as not to
require Python or any other run-time dependencies.
