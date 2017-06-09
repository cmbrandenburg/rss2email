package main

import (
	"crypto/tls"
	"fmt"
	"github.com/boltdb/bolt"
	"github.com/mmcdole/gofeed"
	"html"
	"net/smtp"
	"strings"
	"sync"
	"time"
)

const NUM_FETCHERS = 20
const VERBOSITY = LOG_2

const DEBUG_SEND_TEST = false
const DEBUG_NO_SEND = false

const (
	LOG_0 = iota
	LOG_1
	LOG_2
	LOG_SPEW
)

var logMutex sync.Mutex

func log(verbosity int, format string, a ...interface{}) {
	if VERBOSITY < verbosity {
		return
	}
	line := fmt.Sprintf(format, a...)
	logMutex.Lock()
	defer logMutex.Unlock()
	fmt.Println(line)
}

type mailer struct {
	client *smtp.Client
	config *Config
}

func newMailer(config *Config) (*mailer, error) {

	if DEBUG_NO_SEND {
		return &mailer{}, nil
	}

	c, err := smtp.Dial(config.SmtpServer)
	if err != nil {
		return nil, fmt.Errorf("Failed to connect to SMTP server (server: %q): %v", config.SmtpServer, err)
	}

	tlsConfig := tls.Config{
		ServerName: strings.Split(config.SmtpServer, ":")[0],
	}

	if err := c.StartTLS(&tlsConfig); err != nil {
		return nil, fmt.Errorf("Failed to start TLS with SMTP server (server: %q): %v",
			config.SmtpServer, err)
	}

	auth := smtp.PlainAuth("", config.SmtpUser, config.SmtpPassword, strings.Split(config.SmtpServer, ":")[0])

	if err := c.Auth(auth); err != nil {
		return nil, fmt.Errorf("Failed to authenticate with SMTP server (server: %q, username: %q, password: *****): %v",
			config.SmtpServer, config.SmtpUser, err)
	}

	return &mailer{
		client: c,
		config: config,
	}, nil
}

func (m *mailer) Close() error {

	if DEBUG_NO_SEND {
		return nil
	}

	return m.client.Close()
}

func (m *mailer) send(feed *gofeed.Feed, item *gofeed.Item) error {

	if DEBUG_NO_SEND {
		return nil
	}

	// Format the message.
	// FIXME: Need to escape stuff.

	var parts []string
	parts = append(parts, fmt.Sprintf("To: %v", m.config.Recipient))
	parts = append(parts, fmt.Sprintf("From: %v <%v>", feed.Title, m.config.SmtpUser))
	text := ""
	if DEBUG_SEND_TEST {
		text = "(rss2email-test) "
	}
	parts = append(parts, fmt.Sprintf("Subject: %v%v", text, item.Title))
	parts = append(parts, "Content-Type: text/html")
	parts = append(parts, "")
	parts = append(parts, fmt.Sprintf(`<h1><a href="%v">%v</a></h1>%v<p><a href="%v">%v</a></p>`,
		html.EscapeString(item.Link),
		html.EscapeString(item.Title),
		item.Description,
		html.EscapeString(item.Link),
		html.EscapeString(item.Link)))

	content := strings.Join(parts, "\r\n")

	if err := m.client.Mail(m.config.SmtpUser); err != nil {
		return fmt.Errorf("Failed to execute SMTP MAIL command: %v", err)
	}

	if err := m.client.Rcpt(m.config.Recipient); err != nil {
		return fmt.Errorf("Failed to execute SMTP RCTP command: %v", err)
	}

	w, err := m.client.Data()
	if err != nil {
		return fmt.Errorf("Failed to execute SMTP DATA command: %v", err)
	}

	if _, err := w.Write([]byte(content)); err != nil {
		return fmt.Errorf("Failed to write content in SMTP DATA command: %v", err)
	}

	if err := w.Close(); err != nil {
		return fmt.Errorf("Failed to close SMTP DATA command writer: %v", err)
	}

	return nil
}

func itemGUID(item *gofeed.Item) []byte {
	if 0 < len(item.GUID) {
		return []byte(item.GUID)
	}
	return []byte(item.Link) // fallback--we need something unique
}

func commandRun(config *Config) error {

	db, err := openDatabase(config)
	if err != nil {
		return err
	}

	defer func() {
		if err := db.Close(); err != nil {
			panic(err)
		}
	}()

	return db.Update(func(tx *bolt.Tx) error {

		// NOTE: Bolt transactions are not thread-safe, hence we must access the
		// database only from this goroutine.

		feedBucket := tx.Bucket([]byte("feed"))
		if feedBucket == nil {
			return fmt.Errorf("Master feed bucket does not exist in database")
		}

		// FETCHING

		var feedLinks []string
		if err := feedBucket.ForEach(func(k, v []byte) error {
			feedLink := string(k)
			feedLinks = append(feedLinks, feedLink)
			return nil
		}); err != nil {
			return err
		}

		type fetchResult struct {
			feedLink string
			feed     *gofeed.Feed
		}

		var fetchGroup sync.WaitGroup
		fetchInQueue := make(chan string)
		fetchOutQueue := make(chan *fetchResult)

		fetchGroup.Add(NUM_FETCHERS)
		for i := 0; i < NUM_FETCHERS; i++ {
			go func() {
				defer fetchGroup.Done()
				for feedLink := range fetchInQueue {
					log(LOG_1, "Fetch: %v", feedLink)
					parser := gofeed.NewParser()
					feed, err := parser.ParseURL(feedLink)
					if err != nil {
						log(LOG_0, "*** Failed to fetch %v: %v.", feedLink, err)
						continue
					}
					fetchOutQueue <- &fetchResult{
						feedLink: feedLink,
						feed:     feed,
					}
				}
			}()
		}

		go func() {
			fetchGroup.Wait()
			close(fetchOutQueue)
		}()

		// The reason we send the feed URLs in the background is because this
		// goroutine must be immediately available to receive fetched items.

		go func() {
			for _, feedLink := range feedLinks {
				fetchInQueue <- feedLink
			}
			close(fetchInQueue)
		}()

		// SENDING

		// Take the stream of fetched items and filter out items we've sent
		// previously. Send the remaining items.
		//
		// Sending is done synchronously so that items will arrive at the SMTP
		// server in order. However, this creates a performance bottleneck.

		type fetchItem struct {
			feedLink string
			itemGUID string
		}

		allFetchedItems := make(map[fetchItem]string)

		mailer, err := newMailer(config)
		if err != nil {
			return err
		}

		defer func() {
			if err := mailer.Close(); err != nil {
				panic(err)
			}
		}()

		for fetch := range fetchOutQueue {

			b := feedBucket.Bucket([]byte(fetch.feedLink))
			if b == nil {
				return fmt.Errorf("Feed bucket does not exist in database (feed: %q)", fetch.feedLink)
			}

			itemBucket := b.Bucket([]byte("item"))
			if itemBucket == nil {
				return fmt.Errorf("Feed item bucket does not exist in database (feed: %q)", fetch.feedLink)
			}

			if 0 == len(fetch.feed.Items) {
				log(LOG_1, "!!! Got zero items for %v", fetch.feedLink)
				continue
			}

			for _, item := range fetch.feed.Items {

				guid := itemGUID(item)

				allFetchedItems[fetchItem{
					feedLink: fetch.feedLink,
					itemGUID: string(guid),
				}] = item.Title

				v := itemBucket.Get(guid)
				if v != nil {
					if _, err := feedItemFromBytes(v); err != nil {
						return err
					}
					log(LOG_2, "Skip: %v (%q)", fetch.feedLink, item.Title)
					continue
				}

				log(LOG_2, "Send: %v (%q)", fetch.feedLink, item.Title)

				if err := mailer.send(fetch.feed, item); err != nil {
					return err
				}

				pubDate := time.Now()
				if item.PublishedParsed != nil {
					pubDate = *item.PublishedParsed
				}
				f := FeedItem{
					PubDate: pubDate,
				}

				v, err := f.toBytes()
				if err != nil {
					return err
				}

				if err := itemBucket.Put(guid, v); err != nil {
					return fmt.Errorf("Failed to write feed item to database: %v", err)
				}
			}
		}

		// TODO:Reap old items from the database.

		/*
			now := time.Now()
			if err := feedBucket.ForEach(func(k, v []byte) error {

				feedLink := string(k)

				b := feedBucket.Bucket(k)
				itemBucket := b.Bucket([]byte("item"))

				var keysToDelete [][]byte
				if err := itemBucket.ForEach(func(k, v []byte) error {

					itemTitle, ok := allFetchedItems[fetchItem{
						feedLink: feedLink,
						itemGUID: string(k),
					}]
					if ok {
						return nil // ignore any item just now fetched--regardless of age
					}

					f, err := feedItemFromBytes(v)
					if err != nil {
						return err
					}

					if now.After(f.PubDate.AddDate(7, 0, 0)) {
						log(LOG_2, "Reap: %v (%q)", feedLink, itemTitle)
						keysToDelete = append(keysToDelete, k)
					}

					return nil
				}); err != nil {
					return err
				}

				for _, k := range keysToDelete {
					if err := itemBucket.Delete(k); err != nil {
						return err
					}
				}

				return nil
			}); err != nil {
				return err
			}
		*/

		return nil
	})
}
