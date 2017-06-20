use {Error, std, toml};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub recipient: String,
    pub smtp_server: String,
    pub smtp_username: String,
    pub smtp_password: String,
}

impl Config {
    pub fn load<P: AsRef<Path>>(config_path: P) -> Result<Self, Error> {

        use std::io::Read;

        let config_path = config_path.as_ref();

        let mut f = std::fs::File::open(config_path)
            .map_err(|e| ((format!("Failed to open config file {:?}", config_path), e)))?;

        let mut content = Vec::new();
        f.read_to_end(&mut content)
            .map_err(|e| ((format!("Failed to read config file {:?}", config_path), e)))?;

        let config = toml::from_slice(&content)
            .map_err(|e| ((format!("Failed to parse config file {:?}", config_path), e)))?;

        Ok(config)
    }
}
