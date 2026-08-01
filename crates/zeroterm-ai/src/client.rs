use serde::Deserialize;

#[derive(Debug)]
pub enum AiError {
    RequestFailed(String),
    ResponseParseFailed(String),
    NotReachable,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::RequestFailed(msg) => write!(f, "AI request failed: {}", msg),
            AiError::ResponseParseFailed(msg) => write!(f, "Failed to parse AI response: {}", msg),
            AiError::NotReachable => write!(f, "AI service not reachable"),
        }
    }
}

impl std::error::Error for AiError {}

fn suggest_prompt(context: &str) -> String {
    format!(
        "Based on this terminal command history, suggest the single most likely next command. \
         Reply with just the command, no explanation:\n\n{}",
        context
    )
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

pub struct AiClient {
    endpoint: String,
    model: String,
    client: reqwest::Client,
}

impl AiClient {
    pub fn new(endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: "llama3.2".to_string(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    pub async fn explain(&self, prompt: &str) -> Result<String, AiError> {
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::RequestFailed(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AiError::RequestFailed(format!(
                "HTTP {}: {}",
                status.as_u16(),
                resp.text().await.unwrap_or_default()
            )));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| AiError::ResponseParseFailed(e.to_string()))?;

        let parsed: OllamaResponse =
            serde_json::from_str(&text).map_err(|e| AiError::ResponseParseFailed(e.to_string()))?;

        Ok(parsed.response)
    }

    pub async fn suggest(&self, context: &str) -> Result<String, AiError> {
        let body = serde_json::json!({
            "model": self.model,
            "prompt": suggest_prompt(context),
            "stream": false,
        });

        let resp = self
            .client
            .post(format!("{}/api/generate", self.endpoint))
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::RequestFailed(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(AiError::RequestFailed(format!(
                "HTTP {}: {}",
                status.as_u16(),
                resp.text().await.unwrap_or_default()
            )));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| AiError::ResponseParseFailed(e.to_string()))?;

        let parsed: OllamaResponse =
            serde_json::from_str(&text).map_err(|e| AiError::ResponseParseFailed(e.to_string()))?;

        Ok(parsed.response)
    }

    pub async fn health(&self) -> bool {
        self.client
            .get(format!("{}/api/tags", self.endpoint))
            .send()
            .await
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::suggest_prompt;

    #[test]
    fn prompt_embeds_history_and_asks_for_one_command() {
        let prompt = suggest_prompt("ls\ncd src\ncargo build");
        assert!(prompt.contains("cargo build"));
        assert!(prompt.contains("terminal command history"));
        assert!(prompt.contains("Reply with just the command"));
        // Single instruction, no explanation requested.
        assert!(prompt.matches("no explanation").count() >= 1);
    }
}
