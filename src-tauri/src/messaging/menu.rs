//! The `/model` menu: favourites, one category per provider, and local models.
//!
//! # Why a menu rather than one list
//!
//! A provider can expose hundreds of models. One keyboard of them is unusable on a
//! phone, and the app already organises the same list by provider with favourites on
//! top -- so the chat says the same thing in the same order rather than inventing a
//! second grouping that can drift from it.
//!
//! # Why the selections travel with the category
//!
//! Telegram caps button data at 64 bytes, so a button carries an index into a list
//! that was built when the menu was drawn. The list lives in the sender's state, not
//! in the callback: a button is a pointer, and resolving it here is what keeps the
//! payload small enough to send at all.

use serde_json::{json, Value};

use crate::messaging::Messaging;

/// One heading in the menu, and the models under it.
#[derive(Clone, Debug)]
pub struct Category {
    pub label: String,
    pub selections: Vec<Value>,
}

/// Every category the menu offers, in the order the settings window lists them.
///
/// Favourites first, because that is what a star means; then each enabled provider
/// by its own name; then local models last, as they are in every picker in the app.
pub fn categories(hub: &Messaging) -> Vec<Category> {
    let endpoints = hub.runtime().endpoints().list();
    let favourites: Vec<String> = hub
        .database()
        .setting("app.model_favorites")
        .unwrap_or_default();

    let mut categories = Vec::new();

    // A favourite is a shortcut, not a move: it is named by its provider because
    // `gpt-4o-mini` under a category called Favourites would say nothing about
    // which of two providers serves it.
    let mut starred: Vec<Value> = Vec::new();
    for key in favourites {
        let Some((endpoint_id, model)) = key.split_once(':') else {
            continue;
        };
        let Some(endpoint) = endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id && endpoint.enabled)
        else {
            continue;
        };
        if !endpoint.models.iter().any(|listed| listed == model) {
            continue;
        }
        starred.push(json!({ "endpointId": endpoint_id, "model": model }));
    }
    if !starred.is_empty() {
        let count = starred.len();
        categories.push(Category {
            label: format!("Favourites ({count})"),
            selections: starred,
        });
    }

    for endpoint in endpoints.iter().filter(|endpoint| endpoint.enabled) {
        if endpoint.models.is_empty() {
            continue;
        }
        categories.push(Category {
            label: format!("{} ({})", endpoint.name, endpoint.models.len()),
            selections: endpoint
                .models
                .iter()
                .map(|model| json!({ "endpointId": endpoint.id, "model": model }))
                .collect(),
        });
    }

    // Stored rows rather than `list`: the menu needs a name, not a file size and a
    // GGUF header read per model every time `/model` is typed.
    let local: Vec<Value> = hub
        .runtime()
        .local_models()
        .stored()
        .into_iter()
        .map(|model| json!({ "localModelId": model.id, "name": model.name }))
        .collect();
    if !local.is_empty() {
        let count = local.len();
        categories.push(Category {
            label: format!("Local ({count})"),
            selections: local,
        });
    }

    categories
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The labels carry the count, so a reader knows how deep a category goes
    /// before opening it.
    #[test]
    fn a_category_label_names_itself_and_its_size() {
        let category = Category {
            label: format!("OpenAI ({})", 3),
            selections: Vec::new(),
        };
        assert_eq!(category.label, "OpenAI (3)");
    }

    /// A selection is the same shape every other feature stores, so one resolver in
    /// the runtime serves the menu, the settings page and the title chain alike.
    #[test]
    fn a_selection_keeps_the_shared_shape() {
        let remote = json!({ "endpointId": "e1", "model": "gpt" });
        assert_eq!(remote["endpointId"], "e1");
        let local = json!({ "localModelId": "abc", "name": "Qwen" });
        assert_eq!(local["localModelId"], "abc");
    }
}
