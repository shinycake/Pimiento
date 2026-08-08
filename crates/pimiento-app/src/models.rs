use crate::*;

/// Model metadata from `get_available_models` used by the model/thinking controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelChoice {
    pub(crate) provider: String,
    pub(crate) id: String,
    /// `None` means the model has no controllable thinking surface.
    pub(crate) thinking_efforts: Option<Vec<String>>,
}

pub(crate) fn thinking_label(v: Option<&serde_json::Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    if let Some(s) = v.get("level").and_then(|x| x.as_str()) {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    if let Some(s) = v.get("thinkingLevel").and_then(|x| x.as_str()) {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_owned());
    }
    None
}

pub(crate) fn load_all_model_choices(
    catalog: &serde_json::Value,
    current: Option<&str>,
) -> Vec<ModelChoice> {
    let mut out = Vec::new();
    if let Some(arr) = catalog.get("models").and_then(|v| v.as_array()) {
        for m in arr {
            if let Some(label) = format_model_label(m)
                && let Some((provider, id)) = split_model_label(&label)
            {
                let thinking_efforts = m
                    .get("thinking")
                    .and_then(|thinking| thinking.get("efforts"))
                    .and_then(serde_json::Value::as_array)
                    .map(|efforts| {
                        efforts
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::trim)
                            .filter(|effort| !effort.is_empty())
                            .map(str::to_owned)
                            .collect::<Vec<_>>()
                    })
                    .filter(|efforts| !efforts.is_empty());
                out.push(ModelChoice {
                    provider,
                    id,
                    thinking_efforts,
                });
            }
        }
    }
    let current_choice = current.and_then(split_model_label);
    out.sort_by(|a, b| {
        model_sort_key(a, current_choice.as_ref()).cmp(&model_sort_key(b, current_choice.as_ref()))
    });
    out
}

pub(crate) fn find_model_choice<'a>(
    models: &'a [ModelChoice],
    current_model: Option<&str>,
) -> Option<&'a ModelChoice> {
    let (provider, id) = split_model_label(current_model?)?;
    models
        .iter()
        .find(|choice| choice.provider == provider && choice.id == id)
}

pub(crate) fn thinking_options_for_model(choice: Option<&ModelChoice>) -> Vec<String> {
    let Some(efforts) = choice.and_then(|choice| choice.thinking_efforts.as_ref()) else {
        return Vec::new();
    };
    let mut options = Vec::with_capacity(efforts.len() + 2);
    for option in std::iter::once("off")
        .chain(efforts.iter().map(String::as_str))
        .chain(std::iter::once("auto"))
    {
        if !options.iter().any(|existing| existing == option) {
            options.push(option.to_owned());
        }
    }
    options
}

pub(crate) fn model_matches_query(provider: &str, id: &str, query: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q = q.to_ascii_lowercase();
    provider.to_ascii_lowercase().contains(&q) || id.to_ascii_lowercase().contains(&q)
}

pub(crate) fn model_sort_key(
    choice: &ModelChoice,
    current: Option<&(String, String)>,
) -> (u8, String, String) {
    if current.is_some_and(|(provider, id)| choice.provider == *provider && choice.id == *id) {
        return (0, choice.provider.clone(), choice.id.clone());
    }
    let is_composer_boost = choice.provider == "cursor"
        && (choice.id == "composer-2.5" || choice.id.contains("composer-2.5"));
    let tier = if is_composer_boost {
        1
    } else if choice.provider == "cursor" {
        2
    } else {
        3
    };
    (tier, choice.provider.clone(), choice.id.clone())
}

pub(crate) fn filter_models(models: &[ModelChoice], query: &str) -> Vec<ModelChoice> {
    models
        .iter()
        .filter(|choice| model_matches_query(&choice.provider, &choice.id, query))
        .cloned()
        .collect()
}
