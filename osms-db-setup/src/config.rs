use cfg;
#[derive(Deserialize, Debug)]
pub struct Config {
    pub database_url: String,
    pub username: String,
    pub password: String,
    pub require_tls: bool,
    pub n_threads: usize,
}
impl Config {
    pub fn load() -> Result<Self, ::failure::Error> {
        let mut settings = cfg::Config::default();
        settings
            .merge(cfg::File::with_name("osms-db-setup"))?
            .merge(cfg::Environment::with_prefix("OSMS"))?;
        let ret = settings.try_into()?;
        Ok(ret)
    }
}
