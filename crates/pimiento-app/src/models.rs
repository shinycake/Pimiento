use crate::*;
use std::collections::BTreeMap;
use std::process::Command;

/// Model metadata from `get_available_models` used by the model/thinking controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelChoice {
    pub(crate) provider: String,
    pub(crate) id: String,
    /// Provider API discriminant from the catalog (e.g. `anthropic-messages`).
    pub(crate) api: Option<String>,
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
                let api = m
                    .get("api")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|api| !api.is_empty())
                    .map(str::to_owned);
                out.push(ModelChoice {
                    provider,
                    id,
                    api,
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

/// Mirrors OMP `serviceTierFamily`: `/fast` only exists for `OpenAI` / `Google` /
/// Anthropic(`-messages`) families. Cursor/Grok and most other providers have no
/// service-tier knob, so `set_fast_mode` returns unavailable.
pub(crate) fn model_supports_fast_mode(provider: &str, api: Option<&str>, id: &str) -> bool {
    let provider = provider.to_ascii_lowercase();
    let id = id.to_ascii_lowercase();
    if provider == "openrouter" {
        return id.starts_with("anthropic/")
            || id.starts_with("google/")
            || id.starts_with("openai/");
    }
    if provider == "openai" || provider == "openai-codex" {
        return true;
    }
    if api.is_some_and(|api| api.eq_ignore_ascii_case("anthropic-messages")) {
        return true;
    }
    if provider == "google" || provider == "google-vertex" {
        return true;
    }
    false
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

/// OMP theme color names used by `modelTags` / built-in roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OmpRoleColor {
    Success,
    Warning,
    Accent,
    Error,
    Muted,
    Dim,
}

impl OmpRoleColor {
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "accent" | "info" | "primary" => Some(Self::Accent),
            "error" | "danger" => Some(Self::Error),
            "muted" => Some(Self::Muted),
            "dim" | "secondary" => Some(Self::Dim),
            _ => None,
        }
    }
}

/// Role name → model mapping (+ display metadata) from `~/.omp/agent/config.yml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OmpRole {
    pub(crate) name: String,
    pub(crate) display_name: String,
    pub(crate) provider: String,
    pub(crate) id: String,
    pub(crate) color: OmpRoleColor,
}

fn built_in_role_meta(name: &str) -> (String, OmpRoleColor) {
    match name {
        "default" => ("Default".into(), OmpRoleColor::Success),
        "smol" => ("Fast".into(), OmpRoleColor::Warning),
        "slow" => ("Thinking".into(), OmpRoleColor::Accent),
        "vision" => ("Vision".into(), OmpRoleColor::Error),
        "plan" => ("Architect".into(), OmpRoleColor::Muted),
        "designer" => ("Designer".into(), OmpRoleColor::Muted),
        "commit" => ("Commit".into(), OmpRoleColor::Dim),
        "tiny" => ("Tiny".into(), OmpRoleColor::Dim),
        "task" => ("Subtask".into(), OmpRoleColor::Muted),
        "advisor" => ("Advisor".into(), OmpRoleColor::Accent),
        _ => (name.to_owned(), OmpRoleColor::Muted),
    }
}

pub(crate) fn parse_omp_model_roles_yaml(text: &str) -> Vec<OmpRole> {
    let Ok(root) = serde_yaml::from_str::<serde_yaml::Value>(text) else {
        return Vec::new();
    };
    let Some(map) = root
        .get("modelRoles")
        .and_then(serde_yaml::Value::as_mapping)
    else {
        return Vec::new();
    };
    let tags = root
        .get("modelTags")
        .and_then(serde_yaml::Value::as_mapping);
    let mut roles = Vec::new();
    for (key, value) in map {
        let Some(name) = key.as_str().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        let Some(label) = value
            .as_str()
            .map(str::trim)
            .filter(|label| !label.is_empty())
        else {
            continue;
        };
        let Some((provider, id)) = split_model_label(label) else {
            continue;
        };
        let (mut display_name, mut color) = built_in_role_meta(name);
        if let Some(tag) = tags.and_then(|tags| tags.get(serde_yaml::Value::String(name.into()))) {
            if let Some(configured_name) = tag
                .get("name")
                .and_then(serde_yaml::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                configured_name.clone_into(&mut display_name);
            }
            if let Some(configured_color) = tag
                .get("color")
                .and_then(serde_yaml::Value::as_str)
                .and_then(OmpRoleColor::parse)
            {
                color = configured_color;
            }
        }
        roles.push(OmpRole {
            name: name.to_owned(),
            display_name,
            provider,
            id,
            color,
        });
    }
    roles.sort_by(|left, right| left.name.cmp(&right.name));
    roles
}

pub(crate) fn load_omp_roles_from_home(home: Option<&Path>) -> Vec<OmpRole> {
    let Some(home) = home else {
        return Vec::new();
    };
    let path = home.join(".omp/agent/config.yml");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_omp_model_roles_yaml(&text)
}

fn read_model_roles_map_via_omp() -> Result<BTreeMap<String, String>, String> {
    let output = Command::new("omp")
        .args(["config", "get", "modelRoles", "--json"])
        .output()
        .map_err(|err| format!("omp config get failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "omp config get modelRoles: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|err| format!("omp config get JSON: {err}"))?;
    let value = parsed.get("value").cloned().unwrap_or(parsed);
    let Some(obj) = value.as_object() else {
        return Ok(BTreeMap::new());
    };
    let mut map = BTreeMap::new();
    for (key, val) in obj {
        if let Some(label) = val
            .as_str()
            .map(str::trim)
            .filter(|label| !label.is_empty())
        {
            map.insert(key.clone(), label.to_owned());
        }
    }
    Ok(map)
}

/// Assign `provider/id` to a role via `omp config set modelRoles` (full-record merge).
/// This is the OMP-supported write path — Pimiento never hand-edits YAML.
pub(crate) fn assign_omp_model_role(role: &str, provider: &str, id: &str) -> Result<(), String> {
    let role = role.trim();
    if role.is_empty() {
        return Err("empty role name".into());
    }
    let label = format!("{provider}/{id}");
    let mut map = read_model_roles_map_via_omp()?;
    map.insert(role.to_owned(), label);
    let json = serde_json::to_string(&map).map_err(|err| err.to_string())?;
    let output = Command::new("omp")
        .args(["config", "set", "modelRoles", &json])
        .output()
        .map_err(|err| format!("omp config set failed: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "omp config set modelRoles: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

/// Pending composer attachment: vision `ImageContent` or an `@path` mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingAttachment {
    Image {
        path: Option<PathBuf>,
        mime: String,
        width: u32,
        height: u32,
        data_b64: String,
        label: String,
        /// 1-based `Image #N` marker index.
        marker_index: usize,
    },
    PathMention {
        path: PathBuf,
        /// Text inserted into the composer (e.g. `@/abs/path`).
        display: String,
    },
}

impl PendingAttachment {
    pub(crate) fn chip_label(&self) -> &str {
        match self {
            Self::Image { label, .. } => label.as_str(),
            Self::PathMention { display, .. } => display.as_str(),
        }
    }

    pub(crate) fn matches_path(&self, other: &Path) -> bool {
        match self {
            Self::Image { path: None, .. } => false,
            Self::Image {
                path: Some(path), ..
            }
            | Self::PathMention { path, .. } => path == other,
        }
    }

    pub(crate) fn is_image(&self) -> bool {
        matches!(self, Self::Image { .. })
    }
}

/// OMP `SUPPORTED_IMAGE_MIME_TYPES`: png / jpeg / gif / webp (no BMP).
pub(crate) fn image_mime_for_path(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

pub(crate) fn is_supported_image_path(path: &Path) -> bool {
    image_mime_for_path(path).is_some()
}

/// Anthropic internal vision cap; OMP `image-resize.ts` default max edge.
const MAX_IMAGE_EDGE_PX: u32 = 1568;
/// Smallest edge vision backends reliably accept (OMP / Anthropic 200px floor).
const MIN_IMAGE_EDGE_PX: u32 = 200;
/// Target compressed raw byte budget (not base64) — OMP `DEFAULT_MAX_BYTES`.
const TARGET_MAX_RAW_BYTES: usize = 500 * 1024;
/// OMP `MAX_IMAGE_INPUT_BYTES`.
const MAX_IMAGE_READ_BYTES: usize = 20 * 1024 * 1024;
/// Leave headroom under the 1 MiB unchunked client→server frame limit (PLAN).
const MAX_IMAGE_WIRE_B64_BYTES: usize = 700_000;
const DEFAULT_JPEG_QUALITY: u8 = 80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncodedImage {
    pub(crate) mime: String,
    pub(crate) data_b64: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

fn b64_encode(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

fn webp_excluded() -> bool {
    match std::env::var("OMP_NO_WEBP") {
        Ok(raw) => {
            let v = raw.to_ascii_lowercase();
            v == "1" || v == "true"
        }
        Err(_) => false,
    }
}

fn peek_omp_agent_config_yaml() -> Option<serde_yaml::Value> {
    let path = omp_agent_dir()?.join("config.yml");
    let text = std::fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&text).ok()
}

fn yaml_get_path<'a>(
    root: &'a serde_yaml::Value,
    segments: &[&str],
) -> Option<&'a serde_yaml::Value> {
    let mut cur = root;
    for segment in segments {
        cur = match cur {
            serde_yaml::Value::Mapping(map) => {
                map.get(serde_yaml::Value::String((*segment).into()))?
            }
            _ => return None,
        };
    }
    Some(cur)
}

fn yaml_bool_at(root: &serde_yaml::Value, nested: &[&str], dotted: &str) -> Option<bool> {
    if let Some(v) = yaml_get_path(root, nested).and_then(serde_yaml::Value::as_bool) {
        return Some(v);
    }
    if let Some(v) = root
        .as_mapping()
        .and_then(|map| map.get(serde_yaml::Value::String(dotted.into())))
        .and_then(serde_yaml::Value::as_bool)
    {
        return Some(v);
    }
    None
}

fn yaml_usize_at(root: &serde_yaml::Value, nested: &[&str], dotted: &str) -> Option<usize> {
    let as_usize = |v: &serde_yaml::Value| -> Option<usize> {
        if let Some(n) = v.as_u64() {
            return usize::try_from(n).ok();
        }
        if let Some(n) = v.as_i64() {
            return usize::try_from(n).ok();
        }
        None
    };
    if let Some(v) = yaml_get_path(root, nested).and_then(as_usize) {
        return Some(v);
    }
    root.as_mapping()
        .and_then(|map| map.get(serde_yaml::Value::String(dotted.into())))
        .and_then(as_usize)
}

/// Peek `images.autoResize` from `~/.omp/agent/config.yml` (default `true`).
pub(crate) fn images_auto_resize_enabled() -> bool {
    peek_omp_agent_config_yaml()
        .and_then(|root| yaml_bool_at(&root, &["images", "autoResize"], "images.autoResize"))
        .unwrap_or(true)
}

/// Peek `paste.largeMenuThreshold` (default 100; `0` disables the large-paste menu).
pub(crate) fn large_paste_threshold() -> usize {
    peek_omp_agent_config_yaml()
        .and_then(|root| {
            yaml_usize_at(
                &root,
                &["paste", "largeMenuThreshold"],
                "paste.largeMenuThreshold",
            )
        })
        .unwrap_or(100)
}

pub(crate) fn wrap_attachment(text: &str) -> String {
    format!("<attachment>\n{text}\n</attachment>")
}

/// OMP editor paste marker: `+N lines` when multi-line-ish, else char count.
pub(crate) fn inline_paste_marker(n: usize, lines: usize, chars: usize) -> String {
    if lines > 10 {
        format!("[Paste #{n}, +{lines} lines]")
    } else {
        format!("[Paste #{n}, {chars} chars]")
    }
}

pub(crate) fn image_marker(index: usize, width: u32, height: u32) -> String {
    format!("[Image #{index}, {width}x{height}]")
}

pub(crate) fn image_marker_present(text: &str, index: usize) -> bool {
    let bare = format!("[Image #{index}");
    text.contains(&bare)
}

/// Ensure each pending image has a positional `[Image #N, WxH]` marker in `text`.
pub(crate) fn compose_message_with_image_markers(
    text: &str,
    attachments: &[PendingAttachment],
) -> String {
    let mut out = text.to_owned();
    let mut missing = Vec::new();
    for attachment in attachments {
        let PendingAttachment::Image {
            marker_index,
            width,
            height,
            ..
        } = attachment
        else {
            continue;
        };
        if !image_marker_present(&out, *marker_index) {
            missing.push(image_marker(*marker_index, *width, *height));
        }
    }
    if missing.is_empty() {
        return out;
    }
    if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
        out.push(' ');
    }
    out.push_str(&missing.join(" "));
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        image::ImageFormat::Png => Some("image/png"),
        image::ImageFormat::Jpeg => Some("image/jpeg"),
        image::ImageFormat::Gif => Some("image/gif"),
        image::ImageFormat::WebP => Some("image/webp"),
        _ => None,
    }
}

fn encode_png(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|err| format!("png encode: {err}"))?;
    Ok(out)
}

fn encode_jpeg(img: &image::DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut out);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(|err| format!("jpeg encode: {err}"))?;
    Ok(out)
}

fn encode_webp_lossless(img: &image::DynamicImage) -> Result<Vec<u8>, String> {
    let rgba = img.to_rgba8();
    let mut out = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut out)
        .encode(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|err| format!("webp encode: {err}"))?;
    Ok(out)
}

fn pick_smallest(candidates: Vec<(Vec<u8>, &'static str)>) -> Option<(Vec<u8>, &'static str)> {
    candidates.into_iter().min_by_key(|(buf, _)| buf.len())
}

fn encode_smallest(
    img: &image::DynamicImage,
    quality: u8,
    exclude_webp: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    let mut candidates = Vec::with_capacity(3);
    candidates.push((encode_png(img)?, "image/png"));
    candidates.push((encode_jpeg(img, quality)?, "image/jpeg"));
    if !exclude_webp {
        candidates.push((encode_webp_lossless(img)?, "image/webp"));
    }
    pick_smallest(candidates).ok_or_else(|| "no image encode candidates".to_owned())
}

fn encode_lossy(
    img: &image::DynamicImage,
    quality: u8,
    exclude_webp: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    let mut candidates = Vec::with_capacity(2);
    candidates.push((encode_jpeg(img, quality)?, "image/jpeg"));
    // image crate WebP is lossless-only; still useful as an alternate candidate.
    if !exclude_webp {
        candidates.push((encode_webp_lossless(img)?, "image/webp"));
    }
    pick_smallest(candidates).ok_or_else(|| "no lossy encode candidates".to_owned())
}

/// OMP pixel math: scale/round image edges into `u32` pixels.
fn px_u32_from_u64(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX).max(1)
}

/// OMP pixel math: round a scaled edge into `u32` pixels (values are vision dims).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "intentional OMP image-resize pixel rounding"
)]
fn px_u32_from_f64(v: f64) -> u32 {
    v.round().clamp(1.0, f64::from(u32::MAX)) as u32
}

fn fit_target_dimensions(width: u32, height: u32, max_edge: u32, min_edge: u32) -> (u32, u32) {
    let mut target_w = width.max(1);
    let mut target_h = height.max(1);
    if target_w > max_edge {
        target_h =
            px_u32_from_u64((u64::from(target_h) * u64::from(max_edge)) / u64::from(target_w));
        target_w = max_edge;
    }
    if target_h > max_edge {
        target_w =
            px_u32_from_u64((u64::from(target_w) * u64::from(max_edge)) / u64::from(target_h));
        target_h = max_edge;
    }
    let min_dimension = min_edge.min(max_edge);
    if target_w < min_dimension || target_h < min_dimension {
        let short_edge = target_w.min(target_h).max(1);
        let upscale = (f64::from(min_dimension) / f64::from(short_edge))
            .min(f64::from(max_edge) / f64::from(target_w))
            .min(f64::from(max_edge) / f64::from(target_h));
        if upscale > 1.0 {
            target_w = px_u32_from_f64(f64::from(target_w) * upscale);
            target_h = px_u32_from_f64(f64::from(target_h) * upscale);
        }
        target_w = target_w.clamp(min_dimension, max_edge);
        target_h = target_h.clamp(min_dimension, max_edge);
    }
    (target_w.max(1), target_h.max(1))
}

fn resize_to(img: &image::DynamicImage, width: u32, height: u32) -> image::DynamicImage {
    if img.width() == width && img.height() == height {
        return img.clone();
    }
    img.resize_exact(width, height, image::imageops::FilterType::Triangle)
}

fn encoded_ok(buf: &[u8], max_bytes: usize) -> bool {
    buf.len() <= max_bytes
}

fn to_encoded(mime: &str, buf: &[u8], width: u32, height: u32) -> EncodedImage {
    EncodedImage {
        mime: mime.to_owned(),
        data_b64: b64_encode(buf),
        width,
        height,
    }
}

/// Resize/recompress per OMP `image-resize.ts`. Honors `images.autoResize` (default true);
/// when false, only clamps images that would exceed the rpc-ui wire budget.
pub(crate) fn encode_image_for_rpc(bytes: &[u8]) -> Result<EncodedImage, String> {
    encode_image_for_rpc_with_auto_resize(bytes, images_auto_resize_enabled())
}

/// Decode + OMP-resize. Prefer keeping original bytes on the comfortable fast path.
pub(crate) fn encode_image_for_rpc_with_auto_resize(
    bytes: &[u8],
    auto_resize: bool,
) -> Result<EncodedImage, String> {
    let source_mime = detect_image_mime(bytes).unwrap_or("application/octet-stream");
    let exclude_webp = webp_excluded();
    let img = image::load_from_memory(bytes).map_err(|err| format!("decode image: {err}"))?;
    let original_w = img.width().max(1);
    let original_h = img.height().max(1);
    let original_size = bytes.len();
    let mime_owned = if source_mime == "application/octet-stream" {
        "image/png".to_owned()
    } else {
        source_mime.to_owned()
    };

    if !auto_resize {
        let must_reencode_webp = exclude_webp && source_mime == "image/webp";
        let b64 = b64_encode(bytes);
        if !must_reencode_webp && b64.len() <= MAX_IMAGE_WIRE_B64_BYTES {
            return Ok(EncodedImage {
                mime: mime_owned,
                data_b64: b64,
                width: original_w,
                height: original_h,
            });
        }
        // Over wire budget (or excluded WebP): compress targeting raw bytes that
        // keep base64 under the frame headroom (~700KB b64 ≈ 525KB raw).
        let wire_raw_budget = (MAX_IMAGE_WIRE_B64_BYTES * 3) / 4;
        return compress_image_ladder(
            &img,
            TARGET_MAX_RAW_BYTES.min(wire_raw_budget),
            exclude_webp,
        );
    }

    let min_dimension = MIN_IMAGE_EDGE_PX.min(MAX_IMAGE_EDGE_PX);
    let comfortable = TARGET_MAX_RAW_BYTES / 4;
    if original_w >= min_dimension
        && original_h >= min_dimension
        && original_w <= MAX_IMAGE_EDGE_PX
        && original_h <= MAX_IMAGE_EDGE_PX
        && original_size <= comfortable
        && !(exclude_webp && source_mime == "image/webp")
    {
        return Ok(EncodedImage {
            mime: mime_owned,
            data_b64: b64_encode(bytes),
            width: original_w,
            height: original_h,
        });
    }

    compress_image_ladder(&img, TARGET_MAX_RAW_BYTES, exclude_webp)
}

fn compress_image_ladder(
    img: &image::DynamicImage,
    max_bytes: usize,
    exclude_webp: bool,
) -> Result<EncodedImage, String> {
    let original_w = img.width().max(1);
    let original_h = img.height().max(1);
    let (target_w, target_h) =
        fit_target_dimensions(original_w, original_h, MAX_IMAGE_EDGE_PX, MIN_IMAGE_EDGE_PX);
    let quality_steps: [u8; 4] = [70, 60, 50, 40];
    let scale_steps: [f64; 5] = [1.0, 0.75, 0.5, 0.35, 0.25];

    let sized = resize_to(img, target_w, target_h);
    let first = encode_smallest(&sized, DEFAULT_JPEG_QUALITY, exclude_webp)?;
    let mut final_w = target_w;
    let mut final_h = target_h;
    if encoded_ok(&first.0, max_bytes) {
        return Ok(to_encoded(first.1, &first.0, final_w, final_h));
    }
    let mut best_buf = first;

    for quality in quality_steps {
        let candidate = encode_lossy(&sized, quality, exclude_webp)?;
        if candidate.0.len() < best_buf.0.len() {
            best_buf = (candidate.0.clone(), candidate.1);
        }
        if encoded_ok(&candidate.0, max_bytes) {
            return Ok(to_encoded(candidate.1, &candidate.0, final_w, final_h));
        }
    }

    for scale in scale_steps {
        final_w = px_u32_from_f64(f64::from(target_w) * scale);
        final_h = px_u32_from_f64(f64::from(target_h) * scale);
        if final_w < 100 || final_h < 100 {
            break;
        }
        let scaled = resize_to(img, final_w, final_h);
        for quality in quality_steps {
            let candidate = encode_lossy(&scaled, quality, exclude_webp)?;
            if candidate.0.len() < best_buf.0.len() {
                best_buf = (candidate.0.clone(), candidate.1);
            }
            if encoded_ok(&candidate.0, max_bytes) {
                return Ok(to_encoded(candidate.1, &candidate.0, final_w, final_h));
            }
        }
    }

    let (buf, mime) = best_buf;
    let b64 = b64_encode(&buf);
    if b64.len() > MAX_IMAGE_WIRE_B64_BYTES {
        return Err(format!(
            "image still too large for rpc-ui after compress (limit {MAX_IMAGE_WIRE_B64_BYTES} base64 bytes)"
        ));
    }
    Ok(EncodedImage {
        mime: mime.to_owned(),
        data_b64: b64,
        width: final_w,
        height: final_h,
    })
}

pub(crate) fn load_pending_attachment(
    path: &Path,
    next_image_index: usize,
) -> Result<PendingAttachment, String> {
    if is_supported_image_path(path) {
        let meta = std::fs::metadata(path).map_err(|err| err.to_string())?;
        if meta.len() > u64::try_from(MAX_IMAGE_READ_BYTES).unwrap_or(u64::MAX) {
            return Err(format!(
                "image exceeds {} MiB",
                MAX_IMAGE_READ_BYTES / (1024 * 1024)
            ));
        }
        let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
        if bytes.len() > MAX_IMAGE_READ_BYTES {
            return Err(format!(
                "image exceeds {} MiB",
                MAX_IMAGE_READ_BYTES / (1024 * 1024)
            ));
        }
        let encoded = encode_image_for_rpc(&bytes)?;
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
            .to_owned();
        let marker_index = next_image_index.max(1);
        return Ok(PendingAttachment::Image {
            path: Some(path.to_owned()),
            mime: encoded.mime,
            width: encoded.width,
            height: encoded.height,
            data_b64: encoded.data_b64,
            label,
            marker_index,
        });
    }

    let display = format!("@{}", path.display());
    Ok(PendingAttachment::PathMention {
        path: path.to_owned(),
        display,
    })
}

/// Attach in-memory image bytes (clipboard paste) as a pending image.
pub(crate) fn load_pending_image_bytes(
    bytes: &[u8],
    next_image_index: usize,
    label: &str,
) -> Result<PendingAttachment, String> {
    if bytes.len() > MAX_IMAGE_READ_BYTES {
        return Err(format!(
            "image exceeds {} MiB",
            MAX_IMAGE_READ_BYTES / (1024 * 1024)
        ));
    }
    let encoded = encode_image_for_rpc(bytes)?;
    Ok(PendingAttachment::Image {
        path: None,
        mime: encoded.mime,
        width: encoded.width,
        height: encoded.height,
        data_b64: encoded.data_b64,
        label: label.to_owned(),
        marker_index: next_image_index.max(1),
    })
}

/// Wire `ImageContent` parts — only image variants (path mentions stay in message text).
pub(crate) fn pending_images_to_wire(attachments: &[PendingAttachment]) -> Vec<serde_json::Value> {
    attachments
        .iter()
        .filter_map(|attachment| match attachment {
            PendingAttachment::Image { mime, data_b64, .. } => Some(serde_json::json!({
                "type": "image",
                "mimeType": mime,
                "data": data_b64,
            })),
            PendingAttachment::PathMention { .. } => None,
        })
        .collect()
}

pub(crate) fn next_image_marker_index(attachments: &[PendingAttachment]) -> usize {
    attachments
        .iter()
        .filter_map(|attachment| match attachment {
            PendingAttachment::Image { marker_index, .. } => Some(*marker_index),
            PendingAttachment::PathMention { .. } => None,
        })
        .max()
        .unwrap_or(0)
        .saturating_add(1)
        .max(1)
}

pub(crate) fn path_mention_display(path: &Path, cwd: Option<&Path>) -> String {
    if let Some(cwd) = cwd
        && let Ok(rel) = path.strip_prefix(cwd)
    {
        return format!("@{}", rel.display());
    }
    format!("@{}", path.display())
}

/// Ensure path-mention `@…` tokens are present alongside image markers.
pub(crate) fn compose_message_with_attachments(
    text: &str,
    attachments: &[PendingAttachment],
) -> String {
    let mut out = compose_message_with_image_markers(text, attachments);
    for attachment in attachments {
        let PendingAttachment::PathMention { display, .. } = attachment else {
            continue;
        };
        if out.contains(display.as_str()) {
            continue;
        }
        if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('\n') {
            out.push(' ');
        }
        out.push_str(display);
        out.push(' ');
    }
    out
}

pub(crate) fn strip_image_marker(text: &str, index: usize) -> String {
    let prefix = format!("[Image #{index}");
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(&prefix) {
        out.push_str(&rest[..start]);
        let after_prefix = &rest[start + prefix.len()..];
        if let Some(end) = after_prefix.find(']') {
            let mut consume = end + 1;
            let tail = &after_prefix[consume..];
            if tail.starts_with(' ') {
                consume += 1;
            }
            rest = &after_prefix[consume..];
        } else {
            out.push_str(&prefix);
            rest = after_prefix;
            break;
        }
    }
    out.push_str(rest);
    out
}

pub(crate) fn strip_path_mention(text: &str, display: &str) -> String {
    if display.is_empty() {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    if let Some(pos) = out.find(display) {
        let end = pos + display.len();
        let mut remove_end = end;
        if out[end..].starts_with(' ') {
            remove_end += 1;
        }
        out.replace_range(pos..remove_end, "");
    }
    out
}

/// Open `@token` at end of draft (not mid-email). Returns query after `@`.
pub(crate) fn at_mention_query(text: &str) -> Option<&str> {
    let at = text.rfind('@')?;
    let before = text[..at].chars().next_back();
    if before.is_some_and(|c| !c.is_whitespace() && c != '(' && c != '[' && c != '{') {
        return None;
    }
    let after = &text[at + 1..];
    if after.chars().any(char::is_whitespace) {
        return None;
    }
    Some(after)
}

pub(crate) fn replace_at_mention_token(text: &str, insertion: &str) -> String {
    let Some(at) = text.rfind('@') else {
        let mut out = text.to_owned();
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(insertion);
        if !out.ends_with(' ') {
            out.push(' ');
        }
        return out;
    };
    let mut out = text[..at].to_owned();
    out.push_str(insertion);
    if !out.ends_with(' ') {
        out.push(' ');
    }
    out
}

const AT_FILE_WALK_CAP: usize = 200;
const AT_FILE_MAX_DEPTH: usize = 4;

fn should_skip_at_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | ".omp" | ".pimiento" | "dist" | "build" | ".next"
    )
}

/// Shallow recursive file listing under `cwd`, filtered by `query` (case-insensitive path contains).
pub(crate) fn list_cwd_files_for_at_mention(cwd: &Path, query: &str) -> Vec<PathBuf> {
    let query = query.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut stack = vec![(cwd.to_owned(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= AT_FILE_WALK_CAP || depth > AT_FILE_MAX_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut dirs = Vec::new();
        for entry in entries.flatten() {
            if out.len() >= AT_FILE_WALK_CAP {
                break;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') && name != ".env" {
                // still allow walking? skip hidden dirs except we already filter known ones
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                if should_skip_at_dir(&name) {
                    continue;
                }
                if depth < AT_FILE_MAX_DEPTH {
                    dirs.push(path);
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let rel = path.strip_prefix(cwd).map_or_else(
                |_| path.to_string_lossy().to_ascii_lowercase(),
                |p| p.to_string_lossy().to_ascii_lowercase(),
            );
            if query.is_empty() || rel.contains(&query) {
                out.push(path);
            }
        }
        // Prefer shallower files: push dirs after so files at this level were considered first.
        for d in dirs.into_iter().rev() {
            stack.push((d, depth + 1));
        }
    }
    out.sort();
    out.truncate(AT_FILE_WALK_CAP);
    out
}

/// Parse clipboard / paste text into existing filesystem paths when it looks like path paste.
pub(crate) fn paths_from_paste_text(text: &str) -> Vec<PathBuf> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut paths = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // macOS file URLs may be percent-encoded
        let candidate = line
            .strip_prefix("file://")
            .map_or_else(|| PathBuf::from(line), percent_decode_path);
        if candidate.exists() {
            paths.push(candidate);
        }
    }
    if paths.is_empty() && !trimmed.contains('\n') && Path::new(trimmed).exists() {
        paths.push(PathBuf::from(trimmed));
    }
    paths
}

fn percent_decode_path(raw: &str) -> PathBuf {
    let mut out = String::with_capacity(raw.len());
    let bytes = raw.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = |c: u8| -> Option<u8> {
                match c {
                    b'0'..=b'9' => Some(c - b'0'),
                    b'a'..=b'f' => Some(c - b'a' + 10),
                    b'A'..=b'F' => Some(c - b'A' + 10),
                    _ => None,
                }
            };
            if let (Some(a), Some(b)) = (h(bytes[i + 1]), h(bytes[i + 2])) {
                out.push((a << 4 | b) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    PathBuf::from(out)
}

/// OMP local:// root: `{session.jsonl minus suffix}/local`, else temp.
pub(crate) fn omp_local_paste_dir(session_file: Option<&str>) -> PathBuf {
    if let Some(session_file) = session_file.map(str::trim).filter(|s| !s.is_empty()) {
        let base = session_file.strip_suffix(".jsonl").unwrap_or(session_file);
        return PathBuf::from(base).join("local");
    }
    std::env::temp_dir().join("omp-local").join("pimiento")
}

pub(crate) fn count_text_lines(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.lines().count().max(1)
}

/// Summarize `fileMention` agent messages for transcript rendering.
pub(crate) fn format_file_mention_summary(raw: &serde_json::Value) -> Option<String> {
    if raw.get("role").and_then(|v| v.as_str()) != Some("fileMention") {
        return None;
    }
    let files = raw.get("files").and_then(|v| v.as_array())?;
    let mut parts = Vec::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("(file)");
        if let Some(reason) = file.get("skippedReason").and_then(|v| v.as_str()) {
            parts.push(format!("{path} (skipped: {reason})"));
        } else if file.get("image").is_some() {
            parts.push(format!("{path} (image)"));
        } else if let Some(lines) = file.get("lineCount").and_then(serde_json::Value::as_u64) {
            parts.push(format!("{path} ({lines} lines)"));
        } else {
            parts.push(path.to_owned());
        }
    }
    if parts.is_empty() {
        Some("File mention".into())
    } else {
        Some(format!("File mention: {}", parts.join(", ")))
    }
}

pub(crate) fn roles_matching_model<'a>(
    roles: &'a [OmpRole],
    model: Option<&str>,
) -> Vec<&'a OmpRole> {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Vec::new();
    };
    roles
        .iter()
        .filter(|role| {
            let label = format!("{}/{}", role.provider, role.id);
            label == model
                || (model.starts_with("cursor/")
                    && role.provider == "cursor"
                    && role.id == model.strip_prefix("cursor/").unwrap_or(model))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchMessageChoice {
    pub(crate) entry_id: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoginProviderChoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) available: bool,
    pub(crate) authenticated: bool,
}

pub(crate) fn parse_branch_messages(data: Option<&serde_json::Value>) -> Vec<BranchMessageChoice> {
    data.and_then(|v| v.get("messages"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let entry_id = item
                .get("entryId")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_owned();
            let text = item
                .get("text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .trim()
                .to_owned();
            Some(BranchMessageChoice { entry_id, text })
        })
        .collect()
}

pub(crate) fn parse_login_providers(data: Option<&serde_json::Value>) -> Vec<LoginProviderChoice> {
    data.and_then(|v| v.get("providers"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let id = item
                .get("id")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_owned();
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(id.as_str())
                .to_owned();
            Some(LoginProviderChoice {
                id,
                name,
                available: item
                    .get("available")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                authenticated: item
                    .get("authenticated")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

pub(crate) fn branch_message_preview(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return "(empty message)".to_owned();
    }
    let mut out = trimmed.chars().take(max_chars).collect::<String>();
    if trimmed.chars().count() > max_chars {
        out.push('…');
    }
    out.replace('\n', " ")
}

/// Cycle checklist task status for click-to-toggle: open → completed → pending.
pub(crate) fn next_todo_toggle_status(status: &str) -> &'static str {
    match status {
        "completed" | "abandoned" => "pending",
        _ => "completed",
    }
}

/// Mutate `todos_raw` at `(phase_ix, task_ix)` and return the `phases` array for `set_todos`.
pub(crate) fn toggle_todo_in_phases_json(
    raw: &serde_json::Value,
    phase_ix: usize,
    task_ix: usize,
) -> Option<serde_json::Value> {
    let mut phases = raw.as_array().cloned().or_else(|| {
        raw.get("phases")
            .or_else(|| raw.get("todoPhases"))
            .and_then(serde_json::Value::as_array)
            .cloned()
    })?;
    let phase = phases.get_mut(phase_ix)?;
    let tasks = phase.get_mut("tasks")?.as_array_mut()?;
    let task = tasks.get_mut(task_ix)?;
    let obj = task.as_object_mut()?;
    let current = obj
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("pending");
    let next = next_todo_toggle_status(current);
    obj.insert("status".into(), serde_json::Value::String(next.to_owned()));
    if next != "blocked" {
        obj.remove("blocker");
    }
    Some(serde_json::Value::Array(phases))
}

/// Honest inspector extras from `get_state` raw — only fields that are present.
pub(crate) fn inspector_extra_status_lines(raw: Option<&serde_json::Value>) -> Vec<String> {
    let Some(state) = raw else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(queue) = state
        .get("queuedMessageCount")
        .and_then(serde_json::Value::as_u64)
    {
        lines.push(format!("Queue: {queue}"));
    }
    if let Some(count) = state
        .get("messageCount")
        .and_then(serde_json::Value::as_u64)
    {
        lines.push(format!("Messages: {count}"));
    }
    if let Some(tokens) = state.get("tokens").or_else(|| state.get("usage"))
        && let Some(line) = format_tokens_blob(tokens)
    {
        lines.push(line);
    }
    if let Some(cost) = state
        .get("cost")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| state.get("totalCost").and_then(serde_json::Value::as_f64))
        .filter(|c| c.is_finite() && *c > 0.0)
    {
        lines.push(format!("Cost: ${cost:.4}"));
    }
    lines
}

fn format_tokens_blob(tokens: &serde_json::Value) -> Option<String> {
    let input = tokens
        .get("input")
        .or_else(|| tokens.get("in"))
        .and_then(serde_json::Value::as_u64);
    let output = tokens
        .get("output")
        .or_else(|| tokens.get("out"))
        .and_then(serde_json::Value::as_u64);
    let total = tokens.get("total").and_then(serde_json::Value::as_u64);
    match (input, output, total) {
        (Some(i), Some(o), Some(t)) => Some(format!("Tokens: {i} in / {o} out · {t} total")),
        (Some(i), Some(o), None) => Some(format!("Tokens: {i} in / {o} out")),
        (None, None, Some(t)) => Some(format!("Tokens: {t} total")),
        _ => None,
    }
}
