use reqwest::Client;

use super::http_client_builder::HttpClientBuilder;

pub struct HttpClientDefault;

impl HttpClientBuilder for HttpClientDefault {
    fn create(&self) -> Client {
        Client::builder()
            .build()
            .expect("failed to build reqwest client")
    }
}
