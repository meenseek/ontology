use crate::domain::Error;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use std::{
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    str::FromStr,
};

#[derive(Clone)]
pub struct Config {
    pub address: SocketAddrV4,
    pub web_dist: PathBuf,
}
impl Config {
    pub fn from_env() -> Result<Self, Error> {
        let port = std::env::var("ONTOLOGY_PORT")
            .unwrap_or_else(|_| "47831".into())
            .parse()
            .map_err(|_| Error::Invalid)?;
        if port == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            address: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
            web_dist: std::env::var_os("ONTOLOGY_WEB_DIST")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("web/dist")),
        })
    }
    pub fn host(&self) -> String {
        self.address.to_string()
    }
    pub fn origin(&self) -> String {
        format!("http://{}", self.address)
    }
}
pub fn database_options(url: &str) -> Result<PgConnectOptions, Error> {
    let options = PgConnectOptions::from_str(url).map_err(|_| Error::Invalid)?;
    if options.get_host() != "127.0.0.1" || options.get_database().is_none_or(str::is_empty) {
        return Err(Error::Invalid);
    }
    Ok(options.ssl_mode(PgSslMode::Disable))
}
