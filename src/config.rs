//! Configuration lue dans l'environnement.

use std::net::SocketAddr;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub database_url: String,
    /// Adresse publique du service, par exemple `https://stats.exemple.fr`.
    pub base_url: String,
    /// Nombre de mandataires de confiance devant le service. 0 : l'adresse du
    /// visiteur est celle de la connexion ; 1 : c'est la dernière de `X-Forwarded-For`.
    pub trusted_proxies: usize,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let listen = env_or("SYNALYSE_LISTEN", "127.0.0.1:8292");
        let listen = listen.parse().map_err(|_| format!("SYNALYSE_LISTEN invalide : {listen}"))?;
        let database_url =
            std::env::var("SYNALYSE_DATABASE_URL").map_err(|_| "SYNALYSE_DATABASE_URL est obligatoire".to_string())?;
        let base_url = env_or("SYNALYSE_BASE_URL", "http://localhost:8292").trim_end_matches('/').to_string();
        let proxies = env_or("SYNALYSE_TRUSTED_PROXIES", "0");
        let trusted_proxies = proxies.parse().map_err(|_| format!("SYNALYSE_TRUSTED_PROXIES invalide : {proxies}"))?;
        Ok(Self { listen, database_url, base_url, trusted_proxies })
    }

    pub fn secure_cookies(&self) -> bool {
        self.base_url.starts_with("https://")
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
