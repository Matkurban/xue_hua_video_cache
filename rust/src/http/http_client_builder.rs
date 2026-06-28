use reqwest::Client;

pub trait HttpClientBuilder: Send + Sync {
    fn create(&self) -> Client;
}
