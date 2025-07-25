use reqwest::{Client, Url};
use uuid::Uuid;

pub struct Config {
    pub client: Client,
    pub endpoint: Url,
}

impl Config {
    pub fn new(api_token: String, endpoint: String) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("X-API-Version", "2025-07-24".parse().unwrap());
        headers.insert(
            "X-Invocation-ID",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        headers.insert(
            "Authorization",
            format!("Bearer {}", api_token).parse().unwrap(),
        );
        let client = Client::builder().default_headers(headers).build().unwrap();
        let endpoint = Url::parse(&endpoint).expect("Invalid Attune API endpoint");
        Self { client, endpoint }
    }
}
