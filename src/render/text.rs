use anyhow::{anyhow, Context, Result};
use fontdue::{Font, FontSettings, Metrics};
use log::warn;
use std::{
    borrow::Cow,
    io::{Cursor, Read},
    fs,
    path::{Path, PathBuf},
};

const FONT_DATA: &[u8] = include_bytes!("../../assets/LiberationMono-Regular.ttf");

pub struct TextOverlay {
    fonts: Vec<Font>,
    font_size: f32,
}

impl TextOverlay {
    pub fn new(font_size: f32, font_path: Option<&Path>, font_bytes: Option<&[u8]>) -> Self {
        let mut fonts = Vec::new();

        if let Some(font_path) = font_path {
            if let Some(font) = load_font(font_path) {
                fonts.push(font);
            } else {
                warn!(
                    "Failed to load font: {}. Falling back to system fonts.",
                    font_path.display()
                );
            }
        }

        if let Some(font_bytes) = font_bytes {
            if let Some(font) = load_font_from_bytes(font_bytes) {
                fonts.push(font);
            } else {
                warn!("Failed to parse font bytes from --font-url. Falling back to system fonts.");
            }
        }

        if let Some(path) = find_system_font() {
            if let Some(font) = load_font(&path) {
                fonts.push(font);
            }
        }

        fonts.push(load_embedded_font());

        Self { fonts, font_size }
    }

    /// Composite text onto an RGBA pixel buffer at the given position.
    pub fn composite(
        &self,
        pixels: &mut [u8],
        width: u32,
        height: u32,
        text: &str,
        x: u32,
        y: u32,
        color: [u8; 4],
    ) {
        let mut cursor_x = x as f32;
        for ch in text.chars() {
            let (metrics, bitmap) = self.rasterize_with_fallback(ch);
            if metrics.width == 0 || metrics.height == 0 || bitmap.is_empty() {
                cursor_x += metrics.advance_width;
                continue;
            }

            let glyph_x = cursor_x.round() as i32;
            let glyph_y = y as i32 + self.font_size as i32 - metrics.height as i32 - metrics.ymin;

            for gy in 0..metrics.height {
                for gx in 0..metrics.width {
                    let alpha = bitmap[gy * metrics.width + gx];
                    if alpha == 0 {
                        continue;
                    }

                    let px = glyph_x + gx as i32;
                    let py = glyph_y + gy as i32;

                    if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                        continue;
                    }

                    let idx = ((py as u32 * width + px as u32) * 4) as usize;
                    if idx + 3 >= pixels.len() {
                        continue;
                    }

                    let a = alpha as f32 / 255.0 * (color[3] as f32 / 255.0);
                    let inv_a = 1.0 - a;
                    pixels[idx] = (color[0] as f32 * a + pixels[idx] as f32 * inv_a) as u8;
                    pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv_a) as u8;
                    pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv_a) as u8;
                    pixels[idx + 3] = 255;
                }
            }

            cursor_x += metrics.advance_width;
        }
    }

    /// Fill a rectangle on the pixel buffer with the given RGBA color (alpha-blended).
    #[cfg(feature = "subtitles")]
    pub fn fill_rect(
        pixels: &mut [u8],
        width: u32,
        height: u32,
        rx: u32,
        ry: u32,
        rw: u32,
        rh: u32,
        color: [u8; 4],
    ) {
        let a = color[3] as f32 / 255.0;
        if a == 0.0 {
            return;
        }
        let inv_a = 1.0 - a;
        let x_end = (rx + rw).min(width);
        let y_end = (ry + rh).min(height);
        for py in ry..y_end {
            for px in rx..x_end {
                let idx = ((py * width + px) * 4) as usize;
                if idx + 3 >= pixels.len() {
                    continue;
                }
                pixels[idx] = (color[0] as f32 * a + pixels[idx] as f32 * inv_a) as u8;
                pixels[idx + 1] = (color[1] as f32 * a + pixels[idx + 1] as f32 * inv_a) as u8;
                pixels[idx + 2] = (color[2] as f32 * a + pixels[idx + 2] as f32 * inv_a) as u8;
                pixels[idx + 3] = ((color[3] as f32 * a + pixels[idx + 3] as f32 * inv_a) as u8).max(pixels[idx + 3]);
            }
        }
    }

    /// The font size used for rendering, in pixels.
    #[cfg(feature = "subtitles")]
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Measure the line height based on a capital letter glyph.
    pub fn line_height(&self) -> u32 {
        let (metrics, _) = self.rasterize_with_fallback('M');
        if metrics.height == 0 {
            self.font_size as u32
        } else {
            metrics.height as u32
        }
    }

    /// Measure the width of rendered text in pixels.
    pub fn measure_width(&self, text: &str) -> u32 {
        let mut width = 0.0f32;
        for ch in text.chars() {
            let (metrics, _) = self.rasterize_with_fallback(ch);
            width += metrics.advance_width;
        }
        width.ceil() as u32
    }

    fn rasterize_with_fallback(&self, ch: char) -> (Metrics, Vec<u8>) {
        let mut fallback: Option<(Metrics, Vec<u8>)> = None;

        for font in &self.fonts {
            let (metrics, bitmap) = font.rasterize(ch, self.font_size);
            if metrics.width > 0 && metrics.height > 0 && !bitmap.is_empty() {
                return (metrics, bitmap);
            }

            if fallback.is_none() {
                fallback = Some((metrics, bitmap));
            }
        }

        fallback.unwrap_or_else(|| self.fonts[0].rasterize(ch, self.font_size))
    }
}

pub fn load_font_from_url(url: &str) -> Result<Vec<u8>> {
    let google_family = detect_google_fonts_family(url);

    let (body, content_type) = download_url_bytes(url)
        .with_context(|| format!("Failed to download font URL: {url}"))?;

    if is_font_like_resource(&body, content_type.as_deref(), url) {
        return Ok(body);
    }

    let css: Cow<str> = String::from_utf8_lossy(&body);
    let urls = extract_css_urls(&css.as_ref());

    if let Some(ref google_family) = google_family {
        if let Some(font) = fetch_google_fonts_from_repo(&google_family)? {
            return Ok(font);
        }
    }

    if is_css_like(&body, content_type.as_deref()) {
        if let Some(resolved_url) = find_ttf_font_url(&urls, url) {
            let (ttf_body, resolved_type) = download_url_bytes(&resolved_url)
                .with_context(|| format!("Failed to download font from resolved URL: {resolved_url}"))?;
            if is_font_like_resource(&ttf_body, resolved_type.as_deref(), &resolved_url) {
                return Ok(ttf_body);
            }
            anyhow::bail!("Resolved font URL still looks like CSS/stylesheet. Please provide a direct TTF/OTF file URL.");
        }
    } else if !urls.is_empty() {
        if let Some(resolved_url) = find_ttf_font_url(&urls, url) {
            let (ttf_body, resolved_type) = download_url_bytes(&resolved_url)
                .with_context(|| format!("Failed to download font from resolved URL: {resolved_url}"))?;
            if is_font_like_resource(&ttf_body, resolved_type.as_deref(), &resolved_url) {
                return Ok(ttf_body);
            }
            anyhow::bail!("Resolved font URL still looks like CSS/stylesheet. Please provide a direct TTF/OTF file URL.");
        }
    }

    if let Some(family) = google_family.as_ref() {
        let archive = fetch_google_fonts_archive(&family)?;
        if let Some(font) = extract_font_from_zip(&archive, &family)? {
            return Ok(font);
        }
    }

    let seen = urls.join(", ");
    if seen.is_empty() {
        if google_family.is_some() {
            Err(anyhow!(
                "Google Fonts URL did not expose a downloadable TTF/OTF for family {:?}. \
                Google Fonts `fonts.google.com/specimen/...` and CSS API URLs usually return CSS/woff2 only. \
                Please provide a direct font file URL (e.g. .ttf/.otf from Google Fonts repo, Noto CJK mirror, or local path). \
                For Korean, `NotoSansKR-Regular.otf` is a safe choice.",
                google_family
            ))
        } else {
            Err(anyhow!("Font URL returned CSS but no font URLs were found"))
        }
    } else {
        Err(anyhow!(
            "Font URL returned CSS only (woff2/woff or other formats). No TTF/OTF URL found. URLs: {seen}"
        ))
    }
}

fn download_url_bytes(url: &str) -> Result<(Vec<u8>, Option<String>)> {
    let resp = reqwest::blocking::get(url)
        .with_context(|| format!("Request failed for {url}"))?;

    if !resp.status().is_success() {
        anyhow::bail!("Request to {url} failed with status {}", resp.status());
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let bytes = resp.bytes()?.to_vec();
    Ok((bytes, content_type))
}

fn is_css_like(body: &[u8], content_type: Option<&str>) -> bool {
    if let Some(content_type) = content_type {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("text/css") || ct.contains("css") {
            return true;
        }
    }

    let text: Cow<str> = String::from_utf8_lossy(body);
    let lower = text.to_ascii_lowercase();
    lower.contains("@font-face") || lower.contains("url(") || lower.contains("woff2") || lower.contains("format('woff2')")
}

fn is_font_like_resource(body: &[u8], content_type: Option<&str>, url: &str) -> bool {
    if is_font_signature(body) {
        return true;
    }

    if let Some(content_type) = content_type {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("font/") || ct.contains("application/font") || ct.contains("application/octet-stream") {
            return true;
        }
    }

    if let Some(ext) = extension_from_url(url) {
        return matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "woff" | "woff2");
    }

    false
}

fn is_font_signature(body: &[u8]) -> bool {
    if body.len() < 4 {
        return false;
    }

    matches!(
        &body[0..4],
        b"\x00\x01\x00\x00" |
        b"true" |
        b"OTTO" |
        b"ttcf" |
        b"wOFF" |
        b"wOF2"
    )
}

fn extension_from_url(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let path = parsed.path().to_ascii_lowercase();
    path.rsplit('.').next().map(|ext| ext.to_string())
}

fn find_ttf_font_url(css_urls: &[String], base_url: &str) -> Option<String> {
    for url in css_urls {
        if is_ttf_or_otf_url(&url) {
            return if is_absolute_url(&url) {
                Some(url.to_string())
            } else {
                Some(join_with_base(base_url, &url))
            };
        }
    }
    None
}

fn detect_google_fonts_family(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    if !host.ends_with("fonts.googleapis.com") {
        if host.ends_with("fonts.google.com") && parsed.path().starts_with("/specimen/") {
            let family = parsed.path().trim_start_matches("/specimen/").trim();
            if !family.is_empty() {
                return Some(normalize_google_family(family));
            }
        }
        return None;
    }

    for (key, value) in parsed.query_pairs() {
        if key == "family" {
            return Some(normalize_google_family(&value));
        }
    }
    None
}

fn fetch_google_fonts_from_repo(family: &str) -> Result<Option<Vec<u8>>> {
    let candidates = google_font_repo_candidates(family);
    for candidate_url in candidates {
        let (body, content_type) = match download_url_bytes(&candidate_url) {
            Ok(v) => v,
            Err(err) => {
                warn!("Google Fonts raw URL failed: {candidate_url} ({err})");
                continue;
            }
        };

        if is_css_like(&body, content_type.as_deref()) || is_zip_like(&body, content_type.as_deref()) {
            continue;
        }

        if Font::from_bytes(body.clone(), FontSettings::default()).is_ok() {
            return Ok(Some(body));
        }
    }

    Ok(None)
}

fn google_font_repo_candidates(family: &str) -> Vec<String> {
    let folder = normalize_google_family(family)
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>();

    let stem = google_font_family_stem(family);
    let exts = ["ttf", "otf"];
    let suffixes = ["", "-Regular", "-400", "-Bold", "-Light"];
    let wght_suffix = "%5Bwght%5D";

    let mut urls = Vec::new();
    for ext in exts {
        for suffix in suffixes {
            urls.push(format!(
                "https://raw.githubusercontent.com/google/fonts/main/ofl/{}/{}{}.{}",
                folder,
                stem,
                suffix,
                ext
            ));
        }

        urls.push(format!(
            "https://raw.githubusercontent.com/google/fonts/main/ofl/{}/{}{}.{}",
            folder,
            stem,
            wght_suffix,
            ext
        ));
    }

    urls.extend(google_font_noto_cjk_candidates(family));

    urls
}

fn google_font_noto_cjk_candidates(family: &str) -> Vec<String> {
    let normalized = normalize_google_family(family).to_lowercase();
    let stem = google_font_family_stem(family);
    let region = match normalized
        .split_whitespace()
        .last()
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("kr") => "KR",
        Some("jp") => "JP",
        Some("sc") => "SC",
        Some("tc") => "TC",
        Some("hk") => "HK",
        _ => return Vec::new(),
    };

    let weights = ["", "-Regular", "-Thin", "-Light", "-Medium", "-DemiLight", "-Bold", "-Black"];
    let exts = ["otf", "ttf"];

    let mut urls = Vec::new();
    for ext in exts {
        for weight in weights {
            urls.push(format!(
                "https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/SubsetOTF/{}/{}{}.{}",
                region,
                stem,
                weight,
                ext
            ));
        }
    }

    urls
}

fn google_font_family_stem(family: &str) -> String {
    normalize_google_family(family)
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.chars().all(|ch| ch.is_ascii_uppercase()) || part.len() <= 2 {
                part.to_string()
            } else {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => {
                        let mut token = String::new();
                        token.push_str(&first.to_ascii_uppercase().to_string());
                        token.push_str(&chars.as_str().to_ascii_lowercase());
                        token
                    }
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("")
}

fn fetch_google_fonts_archive(family: &str) -> Result<Vec<u8>> {
    let family_query = family.replace(' ', "+");
    let google_download_url = format!(
        "https://fonts.google.com/download?family={}",
        family_query
    );
    let (body, content_type) = download_url_bytes(&google_download_url)
        .with_context(|| format!("Failed to download Google Fonts archive: {google_download_url}"))?;

    if is_zip_like(&body, content_type.as_deref()) {
        return Ok(body);
    }

    let text: Cow<str> = String::from_utf8_lossy(&body);
    let urls = extract_css_urls(&text);
    for candidate in urls {
        if !is_zip_url(&candidate) {
            continue;
        }
        let url = if is_absolute_url(&candidate) {
            candidate
        } else {
            join_with_base(&google_download_url, &candidate)
        };
        let (archive, archive_ct) = download_url_bytes(&url)
            .with_context(|| format!("Failed to download zip archive from Google Fonts page: {url}"))?;
        if is_zip_like(&archive, archive_ct.as_deref()) {
            return Ok(archive);
        }
    }

    Err(anyhow!(
        "Google Fonts download URL did not return a zip archive for family: {family}"
    ))
}

fn normalize_google_family(family: &str) -> String {
    let cleaned = family.trim().trim_matches('"').trim_matches('\'');
    let cleaned = cleaned.replace('+', " ");
    let cleaned = cleaned.replace("%20", " ");

    cleaned
        .split(|ch: char| ch == ':' || ch == ';')
        .next()
        .unwrap_or(cleaned.as_str())
        .to_string()
}

fn is_zip_like(body: &[u8], content_type: Option<&str>) -> bool {
    if let Some(content_type) = content_type {
        let ct = content_type.to_ascii_lowercase();
        if ct.contains("zip") || ct.contains("application/octet-stream") || ct.contains("binary") {
            return true;
        }
    }
    body.starts_with(b"PK\x03\x04")
}

fn is_zip_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.contains(".zip?")
}

fn extract_font_from_zip(bytes: &[u8], family: &str) -> Result<Option<Vec<u8>>> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("Downloaded file is not a valid zip archive")?;

    let mut fallback = None;
    let preferred_family = family.to_ascii_lowercase();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        if file.is_dir() {
            continue;
        }

        let name = file.name().to_ascii_lowercase();
        if name.ends_with(".ttf") || name.ends_with(".otf") || name.ends_with(".ttc") {
            if name.contains(&preferred_family) && fallback.is_none() {
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                return Ok(Some(data));
            }
            if fallback.is_none() {
                let mut data = Vec::new();
                file.read_to_end(&mut data)?;
                fallback = Some(data);
            }
        }
    }

    Ok(fallback)
}

fn extract_css_urls(css: &str) -> Vec<String> {
    let mut urls = Vec::new();
    let mut rest = css.as_bytes();

    while let Some(start) = find_subslice(rest, b"url(") {
        let after = &rest[start + 4..];
        let Some(end_rel) = find_subslice(after, b")") else {
            break;
        };
        let mut token = String::from_utf8_lossy(&after[..end_rel]).trim().to_string();
        token = token.trim_matches(&['"', '\''][..]).to_string();
        token = token.trim_end_matches(',').to_string();
        if !token.is_empty() {
            urls.push(token);
        }
        rest = &after[end_rel + 1..];
    }

    urls
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn is_ttf_or_otf_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let base = lower
        .split(['?', '#'])
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .unwrap_or("");
    matches!(base, "ttf" | "otf" | "ttc")
}

fn is_absolute_url(url: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url.starts_with("//")
}

fn join_with_base(base_url: &str, relative_url: &str) -> String {
    if relative_url.is_empty() || relative_url.starts_with("http://") || relative_url.starts_with("https://") {
        return relative_url.to_string();
    }
    if relative_url.starts_with("//") {
        return format!("https:{relative_url}");
    }
    if let Ok(base) = reqwest::Url::parse(base_url) {
        if let Ok(joined) = base.join(relative_url) {
            return joined.to_string();
        }
    }
    if relative_url.starts_with('/') {
        if let Some((scheme, host)) = split_host(base_url) {
            return format!("{scheme}://{host}{relative_url}");
        }
    }
    if let Some(idx) = base_url.rfind('/') {
        let base = &base_url[..idx + 1];
        return format!("{base}{relative_url}");
    }
    relative_url.to_string()
}

fn split_host(url: &str) -> Option<(String, String)> {
    let mut parts = url.splitn(2, "://");
    let scheme = parts.next()?.to_string();
    let remainder = parts.next()?;
    let host = remainder.split('/').next().unwrap_or("").to_string();
    Some((scheme, host))
}

fn load_font(path: &Path) -> Option<Font> {
    match fs::read(path) {
        Ok(bytes) => load_font_from_bytes(&bytes),
        Err(err) => {
            warn!("Failed to read font {}: {}", path.display(), err);
            None
        }
    }
}

fn load_font_from_bytes(bytes: &[u8]) -> Option<Font> {
    match Font::from_bytes(bytes.to_vec(), FontSettings::default()) {
        Ok(font) => Some(font),
        Err(err) => {
            warn!("Failed to parse font file: {}", err);
            None
        }
    }
}

fn load_embedded_font() -> Font {
    Font::from_bytes(FONT_DATA, FontSettings::default())
        .expect("Failed to load embedded LiberationMono font")
}

fn find_system_font() -> Option<PathBuf> {
    const CANDIDATE_FONTS: &[&str] = &[
        // macOS
        "/System/Library/Fonts/Supplemental/NotoSansCJK-Regular.ttc",
        "/System/Library/Fonts/Apple SD Gothic Neo.ttc",
        "/Library/Fonts/NotoSansKR-Regular.otf",
        // Linux
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansKR-Regular.otf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        // Windows
        "C:\\Windows\\Fonts\\malgun.ttf",
    ];

    CANDIDATE_FONTS
        .iter()
        .map(Path::new)
        .find(|p| p.exists())
        .map(Path::to_path_buf)
}
