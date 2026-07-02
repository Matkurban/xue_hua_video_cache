use std::time::Duration;

use reqwest::Client;

use super::http_client_builder::HttpClientBuilder;

pub struct HttpClientDefault;

impl HttpClientBuilder for HttpClientDefault {
    fn create(&self) -> Client {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest client")
    }
}
