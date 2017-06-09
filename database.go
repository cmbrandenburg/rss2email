package main

import (
	"bytes"
	"encoding/gob"
	"fmt"
	"github.com/boltdb/bolt"
	"os"
	"time"
)

func openDatabase(config *Config) (*bolt.DB, error) {

	if _, err := os.Stat(config.DatabasePath); os.IsNotExist(err) {
		return nil, fmt.Errorf("Database does not exist (path: %q)", config.DatabasePath)
	} else if err != nil {
		return nil, fmt.Errorf("Failed to stat (path: %q): %v", config.DatabasePath, err)
	}

	return openDatabaseImpl(config)
}

func createDatabase(config *Config) (*bolt.DB, error) {

	if _, err := os.Stat(config.DatabasePath); err == nil {
		return nil, fmt.Errorf("Database already exists (path: %q): %v", config.DatabasePath, err)
	} else if !os.IsNotExist(err) {
		return nil, fmt.Errorf("Failed to stat (path: %q): %v", config.DatabasePath, err)
	}

	return openDatabaseImpl(config)
}

func openDatabaseImpl(config *Config) (*bolt.DB, error) {
	options := bolt.Options{
		Timeout: config.DatabaseTimeout,
	}

	db, err := bolt.Open(config.DatabasePath, 0666, &options)
	if err != nil {
		return nil, fmt.Errorf("Failed to open database (path: %q): %v", config.DatabasePath, err)
	}

	pass := false
	defer func() {
		if !pass {
			if err := db.Close(); err != nil {
				panic(err)
			}
		}
	}()

	db.Update(func(tx *bolt.Tx) error {
		buckets := []string{
			"feed",
		}
		for _, b := range buckets {
			if _, err := tx.CreateBucketIfNotExists([]byte(b)); err != nil {
				return fmt.Errorf("Failed to create database bucket (bucket: %q): %v", b, err)
			}
		}
		return nil
	})

	pass = true
	return db, nil
}

type FeedMeta struct {
	Link          string
	LastBuildDate time.Time
}

func feedMetaFromBytes(v []byte) (*FeedMeta, error) {
	b := bytes.NewReader(v)
	d := gob.NewDecoder(b)
	var f FeedMeta
	if err := d.Decode(&f); err != nil {
		return nil, fmt.Errorf("Could not decode database feed meta (bytes: %v): %v", v, err)
	}
	return &f, nil
}

func (f *FeedMeta) toBytes() ([]byte, error) {
	b := &bytes.Buffer{}
	e := gob.NewEncoder(b)
	if err := e.Encode(f); err != nil {
		return nil, fmt.Errorf("Could not encode database feed meta (feed meta: %v): %v", f, err)
	}
	return b.Bytes(), nil
}

type FeedItem struct {
	PubDate time.Time
}

func feedItemFromBytes(v []byte) (*FeedItem, error) {
	b := bytes.NewReader(v)
	d := gob.NewDecoder(b)
	var f FeedItem
	if err := d.Decode(&f); err != nil {
		return nil, fmt.Errorf("Could not decode database feed item (bytes: %v): %v", v, err)
	}
	return &f, nil
}

func (f *FeedItem) toBytes() ([]byte, error) {
	b := &bytes.Buffer{}
	e := gob.NewEncoder(b)
	if err := e.Encode(f); err != nil {
		return nil, fmt.Errorf("Could not encode database feed item (feed item: %v): %v", f, err)
	}
	return b.Bytes(), nil
}
