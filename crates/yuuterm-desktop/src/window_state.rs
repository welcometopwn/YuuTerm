//! Remembers the main window's size, position and maximized state in
//! `window.json` beside the settings file. Nothing else is stored.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex, time::Instant};
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

const MIN_WIDTH: u32 = 900;
const MIN_HEIGHT: u32 = 600;
const MAX_FILE: u64 = 4096;

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub maximized: bool,
}

fn path() -> Option<PathBuf> {
    yuuterm_core::config::path()
        .ok()
        .map(|p| p.with_file_name("window.json"))
}

pub fn load() -> Option<Geometry> {
    let path = path()?;
    if std::fs::metadata(&path).ok()?.len() > MAX_FILE {
        return None;
    }
    let geometry: Geometry = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    plausible(&geometry).then_some(geometry)
}

fn plausible(g: &Geometry) -> bool {
    (MIN_WIDTH..=16_384).contains(&g.width) && (MIN_HEIGHT..=16_384).contains(&g.height)
}

fn store(geometry: &Geometry) {
    let Some(path) = path() else { return };
    let Ok(data) = serde_json::to_vec(geometry) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, data).is_ok() {
        let _ = std::fs::rename(temp, path);
    }
}

/// Applies the stored geometry before the window is shown. A position that is
/// no longer on any monitor (unplugged display) is ignored, so the window never
/// opens out of reach.
pub fn restore(window: &WebviewWindow, geometry: &Geometry) {
    let wanted = PhysicalSize::new(geometry.width, geometry.height);
    // The first call on a window that is not shown yet adds the height of a
    // caption that the undecorated window does not have (measured: +30 px); the
    // second call is exact.
    let _ = window.set_size(wanted);
    let _ = window.set_size(wanted);
    let on_screen = window.available_monitors().is_ok_and(|monitors| {
        monitors.iter().any(|m| {
            let (p, s) = (m.position(), m.size());
            // The title bar area must be reachable: its left part inside a monitor.
            geometry.x + 100 > p.x
                && geometry.x + 100 < p.x + s.width as i32
                && geometry.y >= p.y
                && geometry.y < p.y + s.height as i32 - 50
        })
    });
    if on_screen {
        let _ = window.set_position(PhysicalPosition::new(geometry.x, geometry.y));
    }
    if geometry.maximized {
        let _ = window.maximize();
    }
}

/// The last normal (not maximized, not minimized) geometry, written at most once
/// a second while the window moves or resizes and once more when it closes.
pub struct Tracker {
    last: Mutex<Option<Geometry>>,
    written: Mutex<Instant>,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            last: Mutex::new(None),
            written: Mutex::new(Instant::now()),
        }
    }
}

impl Tracker {
    pub fn update(&self, window: &WebviewWindow, flush: bool) {
        if window.is_minimized().unwrap_or(false) {
            return;
        }
        let maximized = window.is_maximized().unwrap_or(false);
        let (Ok(size), Ok(position)) = (window.inner_size(), window.outer_position()) else {
            return;
        };
        let Ok(mut last) = self.last.lock() else {
            return;
        };
        let mut next = last.unwrap_or(Geometry {
            width: size.width,
            height: size.height,
            x: position.x,
            y: position.y,
            maximized,
        });
        next.maximized = maximized;
        if !maximized {
            next = Geometry {
                width: size.width,
                height: size.height,
                x: position.x,
                y: position.y,
                maximized,
            };
        }
        if !plausible(&next) {
            return;
        }
        *last = Some(next);
        let due = self
            .written
            .lock()
            .map(|mut at| {
                let due = at.elapsed().as_secs() >= 1;
                if due || flush {
                    *at = Instant::now();
                }
                due
            })
            .unwrap_or(false);
        if due || flush {
            store(&next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implausible_sizes_are_rejected() {
        let ok = Geometry {
            width: 1280,
            height: 820,
            x: 10,
            y: 10,
            maximized: false,
        };
        assert!(plausible(&ok));
        assert!(!plausible(&Geometry { width: 0, ..ok }));
        assert!(!plausible(&Geometry { height: 100, ..ok }));
        assert!(!plausible(&Geometry {
            width: 100_000,
            ..ok
        }));
    }

    #[test]
    fn geometry_round_trips_and_rejects_unknown_fields() {
        let g = Geometry {
            width: 1500,
            height: 900,
            x: -20,
            y: 30,
            maximized: true,
        };
        let json = serde_json::to_string(&g).unwrap();
        assert_eq!(serde_json::from_str::<Geometry>(&json).unwrap(), g);
        assert!(serde_json::from_str::<Geometry>(r#"{"width":1,"extra":1}"#).is_err());
    }
}
