//! Asking a vision model what is in an image.
//!
//! `read_file` can pull text out of a document because a document has a text
//! layer. An image has none: the only way to know what is in it is to show it to
//! a model that can see, which is why this is the one part of reading a file that
//! needs a model at all.
//!
//! It lives with the AI tools rather than with `documents/` for exactly that
//! reason -- the readers there are pure functions over bytes, and this is a
//! network call to a model the user chose.

use base64::Engine;
use serde_json::json;

use crate::ai::remote::types::Endpoint;

/// Asks a vision model what is in an image.
///
/// The image travels inline as a base64 data URL, the same shape the composer
/// uses for an attachment, so a provider that already accepted a pasted
/// screenshot accepts one a tool read off disk.
pub async fn describe_image(
    endpoint: &Endpoint,
    model: &str,
    bytes: &[u8],
    media_type: &str,
    question: &str,
) -> Result<String, String> {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let body = json!({
        "model": model,
        "stream": false,
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": question },
                { "type": "image_url", "image_url": {
                    "url": format!("data:{media_type};base64,{encoded}")
                }}
            ]
        }]
    });
    let url = crate::ai::remote::types::chat_url(&endpoint.base_url);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let mut request = client.post(&url).json(&body);
    if !endpoint.api_key.trim().is_empty() {
        request = request.bearer_auth(&endpoint.api_key);
    }
    let response = request
        .send()
        .await
        .map_err(|error| format!("Could not reach the vision model: {error}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let detail = response.text().await.unwrap_or_default();
        return Err(format!(
            "The vision model rejected the image ({status}): {detail}"
        ));
    }
    let payload: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid response from the vision model: {error}"))?;
    let content = payload["choices"][0]["message"]["content"]
        .as_str()
        .or_else(|| payload["choices"][0]["message"]["reasoning_content"].as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("The vision model returned nothing for the image.".to_string());
    }
    Ok(content)
}
