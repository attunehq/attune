use attune::server::compatibility::{API_VERSION_HEADER, API_VERSION_HEADER_V0_2_0};
use derive_more::Debug;
use reqwest::{Client, Url};
use uuid::Uuid;

/// Global configuration for the Attune CLI client.
#[derive(Debug, Clone)]
pub struct Config {
    /// The HTTP client used to make requests to the Attune API.
    #[debug(skip)]
    pub client: Client,

    /// The Attune API root.
    #[debug("{}", endpoint.as_str())]
    pub endpoint: Url,
}

#[bon::bon]
impl Config {
    /// Construct a new configuration.
    #[builder]
    pub fn new(#[builder(into)] api_token: String, #[builder(into)] endpoint: String) -> Self {
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
