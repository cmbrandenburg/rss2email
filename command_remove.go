package main

import (
	"fmt"
	"github.com/boltdb/bolt"
)

func commandRemove(config *Config, url string) error {

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

		if err := b1.DeleteBucket([]byte(url)); err != nil {
			return fmt.Errorf("Failed to remove feed %q from database", url)
		}

		return nil
	})
}
