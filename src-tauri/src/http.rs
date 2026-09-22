use once_cell::sync::Lazy;

const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .user_agent(DEFAULT_UA)
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .expect("failed to build shared reqwest client")
});

pub fn client() -> reqwest::Client {
    HTTP_CLIENT.clone()
}
