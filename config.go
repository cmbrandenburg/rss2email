package main

import (
	"fmt"
	"github.com/BurntSushi/toml"
	"time"
)

type Config struct {
	Recipient       string
	SmtpServer      string
	SmtpUser        string
	SmtpPassword    string
	DatabasePath    string
	DatabaseTimeout time.Duration
}

func loadConfig() (*Config, error) {

	path := "rss2email.toml"

	var config Config
	if _, err := toml.DecodeFile(path, &config); err != nil {
		return nil, fmt.Errorf("Failed to load configuration (path: %q): %v", path, err)
	}

	if config.DatabasePath == "" {
		config.DatabasePath = "rss2email.db"
	}

	config.DatabaseTimeout = time.Second

	return &config, nil
}
