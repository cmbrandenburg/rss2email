package main

import (
	"fmt"
	"github.com/boltdb/bolt"
)

func commandAdd(config *Config, url string) error {

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

		b1 := tx.Bucket([]byte("feed"))
		if b1 == nil {
			panic("Feed bucket in database does not exist")
		}

		if b1.Bucket([]byte(url)) != nil {
			return fmt.Errorf("Feed %q already exists in database", url)
		}

		b2, err := b1.CreateBucket([]byte(url))
		if err != nil {
			return err
		}

		if _, err := b2.CreateBucket([]byte("item")); err != nil {
			return err
		}

		m := &FeedMeta{
			Link: url,
		}
		v, err := m.toBytes()
		if err != nil {
			return err
		}

		if err := b2.Put([]byte("meta"), v); err != nil {
			return err
		}

		return nil
	})
}
