#[allow(dead_code)]
pub struct AiClient {
    endpoint: String,
}

impl AiClient {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
        }
    }
}
