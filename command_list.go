package main

import (
	"fmt"
	"github.com/boltdb/bolt"
)

func commandList(config *Config) error {

	db, err := openDatabase(config)
	if err != nil {
		return err
	}

	defer func() {
		if err := db.Close(); err != nil {
			panic(err)
		}
	}()

	return db.View(func(tx *bolt.Tx) error {

		b1 := tx.Bucket([]byte("feed"))
		if b1 == nil {
			panic("Feed bucket in database does not exist")
		}

		if err := b1.ForEach(func(k1, v1 []byte) error {

			b2 := b1.Bucket(k1)
			if b2 == nil {
				panic("Feed object is not a bucket")
			}

			v2 := b2.Get([]byte("meta"))
			m, err := feedMetaFromBytes(v2)
			if err != nil {
				return err
			}

			fmt.Printf("%v\n", m.Link)

			return nil
		}); err != nil {
			return err
		}

		return nil
	})
}
