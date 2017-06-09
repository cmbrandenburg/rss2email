package main

import (
	"fmt"
	"gopkg.in/alecthomas/kingpin.v2"
	"os"
	"path/filepath"
)

func main() {

	// Set up for command-line parsing.

	app := kingpin.New("rss2email", "Send RSS feeds to email")

	add := app.Command("add", "Add feed to database")
	addURL := add.Arg("URL", "Feed URL").Required().String()

	createDatabase := app.Command("create-database", "Create new database")

	list := app.Command("list", "List feeds in database")

	remove := app.Command("remove", "Remove feed from database")
	removeURL := remove.Arg("URL", "Feed URL").Required().String()

	run := app.Command("run", "Fetch feeds and send email")

	// Everything is wrapped in a top-level error handler.

	if err := func() error {
		switch kingpin.MustParse(app.Parse(os.Args[1:])) {
		case add.FullCommand():
			config, err := loadConfig()
			if err != nil {
				return err
			}
			return commandAdd(config, *addURL)
		case createDatabase.FullCommand():
			config, err := loadConfig()
			if err != nil {
				return err
			}
			return commandCreateDatabase(config)
		case list.FullCommand():
			config, err := loadConfig()
			if err != nil {
				return err
			}
			return commandList(config)
		case remove.FullCommand():
			config, err := loadConfig()
			if err != nil {
				return err
			}
			return commandRemove(config, *removeURL)
		case run.FullCommand():
			config, err := loadConfig()
			if err != nil {
				return err
			}
			return commandRun(config)
		default:
			panic("Unknown command-line command")
		}
		return nil
	}(); err != nil {
		fmt.Fprintf(os.Stderr, "%v: *** %v\n", filepath.Base(os.Args[0]), err)
		os.Exit(1)
	}
}
