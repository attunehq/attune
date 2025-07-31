use attune::server::compatibility::{API_VERSION_HEADER, API_VERSION_HEADER_V0_2_0};
use reqwest::{Client, Url};
use uuid::Uuid;

pub struct Config {
    pub client: Client,
    pub endpoint: Url,
}

impl Config {
    pub fn new(api_token: String, endpoint: String) -> Self {
        // Parse server API endpoint.
        let endpoint = Url::parse(&endpoint).expect("Invalid Attune API endpoint");

        // Set up default headers.
        let mut headers = reqwest::header::HeaderMap::new();

        // We send this as a header so that a future server can route requests
        // based on this header (which gives us more optionality for preserving
        // backwards compatibility).
        headers.insert(
            API_VERSION_HEADER,
            API_VERSION_HEADER_V0_2_0.parse().unwrap(),
        );
        // _Invocations_ are distinct from _requests_ because a single CLI
        // invocation may spawn multiple API requests.
        headers.insert(
            "X-Invocation-ID",
            Uuid::new_v4().to_string().parse().unwrap(),
        );
        headers.insert(
            "Authorization",
            format!("Bearer {api_token}").parse().unwrap(),
        );

        // Build default client.
        let client = Client::builder().default_headers(headers).build().unwrap();
        Self { client, endpoint }
    }
}
