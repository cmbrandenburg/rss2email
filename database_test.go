package main

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func mustCreateTempDir() string {
	if p, err := ioutil.TempDir("", "com.github.cmbrandenburg.rss2email-test"); err != nil {
		panic(err)
	} else {
		return p
	}
}

func mustRemoveTempDir(path string) {
	if err := os.RemoveAll(path); err != nil {
		panic(err)
	}
}

func TestOpenDatabase(t *testing.T) {

	tmpdir := mustCreateTempDir()
	defer mustRemoveTempDir(tmpdir)

	config := &Config{
		DatabasePath:    filepath.Join(tmpdir, "foo.db"),
		DatabaseTimeout: time.Second,
	}

	// Opening a non-existing database is an error.

	if db, err := openDatabase(config); err == nil {
		if err := db.Close(); err != nil {
			panic(err)
		}
		t.Fatal("Got unexpected successful result")
	}

	// Creating a non-existing database succeeds.

	db, err := createDatabase(config)
	if err != nil {
		t.Fatal(err)
	}

	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	// Opening an existing database succeeds.

	db, err = openDatabase(config)
	if err != nil {
		t.Fatal(err)
	}

	if err := db.Close(); err != nil {
		t.Fatal(err)
	}

	// Creating an existing database is an error.

	if db, err := createDatabase(config); err == nil {
		if err := db.Close(); err != nil {
			panic(err)
		}
		t.Fatal("Got unexpected successful result")
	}
}
