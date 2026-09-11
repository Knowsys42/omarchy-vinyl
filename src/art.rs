//! Fetch album art from a `file://` or `http(s)://` URL and decode it to a texture.

use gtk::gdk;
use gtk::gio;
use gtk::glib;

const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// The size Apple's artwork service is asked for when the URL carries one.
const APPLE_ART_SIZE: u32 = 1400;

pub async fn load_texture(url: String) -> Result<gdk::Texture, String> {
    // Try a larger variant first where the service encodes the size in the
    // URL, then fall back to exactly what the player gave us.
    let mut candidates = Vec::with_capacity(2);
    if let Some(bigger) = upgrade_url(&url) {
        candidates.push(bigger);
    }
    candidates.push(url);

    let mut last = String::from("no art candidates");
    for candidate in candidates {
        let fetching = candidate.clone();
        match gio::spawn_blocking(move || fetch(&fetching)).await {
            Ok(Ok(bytes)) => {
                let bytes = glib::Bytes::from_owned(bytes);
                match gdk::Texture::from_bytes(&bytes) {
                    Ok(texture) => return Ok(texture),
                    Err(e) => last = format!("decode {candidate}: {e}"),
                }
            }
            Ok(Err(e)) => last = e,
            Err(_) => last = "art fetch task panicked".to_string(),
        }
    }
    Err(last)
}

/// Several services encode the wanted size in the artwork URL, and what they
/// publish over MPRIS is usually a thumbnail: a browser hands out 150px, which
/// is nothing across a full-screen sleeve. Ask for something large enough.
/// Returns None when the URL is not a pattern we recognise.
pub fn upgrade_url(url: &str) -> Option<String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return None;
    }
    if url.contains("mzstatic.com") {
        if let Some(bigger) = resize_apple(url) {
            return Some(bigger);
        }
    }
    // Spotify encodes the size in the image id: 4851 is 64px, 1e02 is 300px,
    // b273 is 640px, which is the largest that CDN serves.
    if url.contains("i.scdn.co/image/") {
        for small in ["ab67616d00004851", "ab67616d00001e02"] {
            if url.contains(small) {
                return Some(url.replace(small, "ab67616d0000b273"));
            }
        }
    }
    None
}

/// Apple artwork ends in a `<w>x<h>` segment, e.g. `.../300x300bb.jpg`.
fn resize_apple(url: &str) -> Option<String> {
    let (head, last) = url.rsplit_once('/')?;
    let x = last.find('x')?;
    let (width, rest) = last.split_at(x);
    if width.is_empty() || !width.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let rest = &rest[1..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let suffix = &rest[digits..];
    Some(format!("{head}/{APPLE_ART_SIZE}x{APPLE_ART_SIZE}{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_artwork_is_asked_for_at_full_size() {
        assert_eq!(
            upgrade_url("https://is1-ssl.mzstatic.com/image/thumb/abc/300x300bb.jpg").as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/abc/1400x1400bb.jpg")
        );
    }

    #[test]
    fn spotify_thumbnails_are_promoted_to_the_largest_variant() {
        assert_eq!(
            upgrade_url("https://i.scdn.co/image/ab67616d00001e0252e8aa32").as_deref(),
            Some("https://i.scdn.co/image/ab67616d0000b27352e8aa32")
        );
    }

    #[test]
    fn unknown_and_local_urls_are_left_alone() {
        assert_eq!(upgrade_url("file:///tmp/art.png"), None);
        assert_eq!(upgrade_url("https://example.com/cover.jpg"), None);
        assert_eq!(upgrade_url("https://i.scdn.co/image/ab67616d0000b273aa"), None);
    }
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    if url.starts_with("file://") {
        let (path, _) = glib::filename_from_uri(url).map_err(|e| format!("bad file uri: {e}"))?;
        std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        let mut resp = ureq::get(url).call().map_err(|e| format!("http: {e}"))?;
        resp.body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_vec()
            .map_err(|e| format!("http body: {e}"))
    } else if let Some(rest) = url.strip_prefix("data:") {
        // data:[<mediatype>][;base64],<data>
        let (meta, data) = rest.split_once(',').ok_or("malformed data url")?;
        if meta.ends_with(";base64") {
            Ok(glib::base64_decode(data).to_vec())
        } else {
            Ok(data.as_bytes().to_vec())
        }
    } else {
        Err(format!("unsupported art url: {url}"))
    }
}
