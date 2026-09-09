//! Fetch album art from a `file://` or `http(s)://` URL and decode it to a texture.

use gtk::gdk;
use gtk::gio;
use gtk::glib;

const MAX_BYTES: u64 = 20 * 1024 * 1024;

pub async fn load_texture(url: String) -> Result<gdk::Texture, String> {
    let bytes = gio::spawn_blocking(move || fetch(&url))
        .await
        .map_err(|_| "art fetch task panicked".to_string())??;
    let bytes = glib::Bytes::from_owned(bytes);
    gdk::Texture::from_bytes(&bytes).map_err(|e| format!("decode: {e}"))
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
