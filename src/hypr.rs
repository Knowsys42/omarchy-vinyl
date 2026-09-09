//! Talks to Hyprland: queries over `hyprctl -j`, and the event socket for
//! workspace and monitor changes.

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub name: String,
    pub focused: bool,
    pub workspace_id: i64,
    pub workspace_name: String,
}

fn socket_dir() -> Option<PathBuf> {
    let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    Some(PathBuf::from(runtime).join("hypr").join(sig))
}

pub fn available() -> bool {
    socket_dir().map(|d| d.join(".socket2.sock").exists()).unwrap_or(false)
}

async fn hyprctl(args: &[&str]) -> Option<serde_json::Value> {
    let mut argv = vec!["hyprctl", "-j"];
    argv.extend_from_slice(args);
    let proc = gio::Subprocess::newv(
        &argv.iter().map(|s| std::ffi::OsStr::new(s)).collect::<Vec<_>>(),
        gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_SILENCE,
    )
    .ok()?;
    let (out, _) = proc.communicate_utf8_future(None).await.ok()?;
    serde_json::from_str(out?.as_str()).ok()
}

pub async fn monitors() -> Vec<MonitorInfo> {
    let Some(v) = hyprctl(&["monitors"]).await else {
        return Vec::new();
    };
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|m| {
                    Some(MonitorInfo {
                        name: m.get("name")?.as_str()?.to_string(),
                        focused: m.get("focused").and_then(|f| f.as_bool()).unwrap_or(false),
                        workspace_id: m.get("activeWorkspace")?.get("id")?.as_i64()?,
                        workspace_name: m
                            .get("activeWorkspace")?
                            .get("name")?
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The focused monitor's name and active workspace.
pub async fn focused() -> Option<MonitorInfo> {
    monitors().await.into_iter().find(|m| m.focused)
}

/// Call `on_change` (debounced) whenever workspaces or monitors change.
/// Reconnects if Hyprland drops the socket. Returns false if there is no
/// Hyprland event socket to watch.
pub fn watch(on_change: impl Fn() + 'static) -> bool {
    let Some(path) = socket_dir().map(|d| d.join(".socket2.sock")) else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    let on_change = Rc::new(on_change);
    let generation = Rc::new(Cell::new(0u64));
    glib::spawn_future_local(async move {
        loop {
            let client = gio::SocketClient::new();
            let addr = gio::UnixSocketAddress::new(&path);
            let conn = match client.connect_future(&addr).await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("vinyl: hyprland event socket: {e}");
                    glib::timeout_future(Duration::from_secs(3)).await;
                    continue;
                }
            };
            if std::env::var_os("VINYL_DEBUG").is_some() {
                eprintln!("vinyl: hyprland event socket connected");
            }
            let input = gio::DataInputStream::new(&conn.input_stream());
            loop {
                let line = match input.read_line_utf8_future(glib::Priority::DEFAULT).await {
                    Ok(Some(l)) => l,
                    other => {
                        if std::env::var_os("VINYL_DEBUG").is_some() {
                            eprintln!("vinyl: hyprland event socket closed: {other:?}");
                        }
                        break;
                    }
                };
                let relevant = [
                    "workspace", "focusedmon", "moveworkspace", "createworkspace",
                    "destroyworkspace", "renameworkspace", "monitoradded", "monitorremoved",
                    "activespecial",
                ];
                if !relevant.iter().any(|k| line.starts_with(k)) {
                    continue;
                }
                if std::env::var_os("VINYL_DEBUG").is_some() {
                    eprintln!("vinyl: hypr event {line}");
                }
                let gen = generation.get() + 1;
                generation.set(gen);
                let generation = generation.clone();
                let on_change = on_change.clone();
                glib::timeout_add_local_once(Duration::from_millis(80), move || {
                    if generation.get() == gen {
                        on_change();
                    }
                });
            }
            glib::timeout_future(Duration::from_secs(2)).await;
        }
    });
    true
}
