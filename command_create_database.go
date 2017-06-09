package main

func commandCreateDatabase(config *Config) error {
	if db, err := createDatabase(config); err != nil {
		return err
	} else if err := db.Close(); err != nil {
		return err
	}
	return nil
}
