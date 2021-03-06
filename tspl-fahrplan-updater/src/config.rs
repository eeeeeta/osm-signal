/// Standard configuration.

use serde_derive::Deserialize;
use tspl_util::{ConfigExt, crate_name};

#[derive(Deserialize, Debug)]
pub struct Config {
    pub bucket_name: String,
    pub service_account_key_path: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub update_timeout_ms: Option<u32>,
    #[serde(default)]
    pub update_retries: Option<u32>
}

impl ConfigExt for Config {
    fn crate_name() -> &'static str {
        crate_name!()
    }
}
