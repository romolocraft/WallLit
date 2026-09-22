use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::core::Result;
use windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager;

use crate::config;
use crate::poster;
use crate::renderer::{FitMode, Gpu, Placement};

const THUMBNAIL_WIDTH: u32 = 384;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub name: String,

    pub prepared: PathBuf,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,
    pub thumbnail: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    #[serde(default)]
    pub favorite: bool,
}

impl Item {
    pub fn aspect(&self) -> f32 {
        if self.height == 0 {
            16.0 / 9.0
        } else {
            self.width as f32 / self.height as f32
        }
    }

    pub fn detail(&self) -> String {
        if self.fps <= 0.0 {
            format!("{} × {}", self.width, self.height)
        } else {
            format!("{} × {} · {:.0} fps", self.width, self.height, self.fps)
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    items: Vec<Item>,
}

impl Library {
    pub fn load() -> Self {
        Self::try_load().unwrap_or_else(|e| { crate::diagnostics::record(&format!("library: {e}")); Self::default() })
    }

    pub fn try_load() -> std::io::Result<Self> {
        let path = index_path().map_err(|e| std::io::Error::other(e.message()))?;
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(config::without_bom(&text))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn toggle_favorite(&mut self, index: usize) {
        if let Some(item) = self.items.get_mut(index) { item.favorite = !item.favorite; }
    }

    pub fn matching(&self, query: &str, favorites: bool) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        self.items.iter().enumerate().filter(|(_, item)|
            (!favorites || item.favorite) && item.name.to_lowercase().contains(&query)
        ).map(|(index, _)| index).collect()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = index_path().map_err(|e| std::io::Error::other(e.message()))?;

        crate::storage::write_json(&path, self)
    }

    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn prune(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|item| item.prepared.is_file());
        before != self.items.len()
    }

    pub fn add(
        &mut self,
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        prepared: &Path,
        source: Option<&Path>,
        size: (u32, u32),
        fps: f32,
    ) -> Result<()> {
        let name = source
            .unwrap_or(prepared)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "wallpaper".into());

        let thumbnail = make_thumbnail(gpu, manager, prepared, size)?;

        let item = Item {
            name,
            prepared: prepared.to_path_buf(),
            source: source.map(Path::to_path_buf),
            thumbnail,
            width: size.0,
            height: size.1,
            fps,
            favorite: self.items.iter().find(|i| i.prepared == prepared).is_some_and(|i| i.favorite),
        };

        match self.items.iter_mut().find(|i| i.prepared == item.prepared) {
            Some(existing) => *existing = item,

            None => self.items.insert(0, item),
        }

        Ok(())
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.items.len() {
            return;
        }

        self.items.remove(index);
    }
}

pub fn index_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join("library.json"))
}

fn thumbnail_dir() -> Result<PathBuf> {
    Ok(config::data_dir()?.join("thumbs"))
}

fn make_thumbnail(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    video: &Path,
    size: (u32, u32),
) -> Result<PathBuf> {
    let directory = thumbnail_dir()?;
    std::fs::create_dir_all(&directory).map_err(poster::to_windows_error)?;

    let aspect = if size.1 == 0 {
        16.0 / 9.0
    } else {
        size.0 as f32 / size.1 as f32
    };
    let width = THUMBNAIL_WIDTH.min((THUMBNAIL_WIDTH as f32 * aspect).round().max(2.0) as u32);
    let height = ((width as f32 / aspect).round() as u32).clamp(2, THUMBNAIL_WIDTH);

    let pixels = if crate::image::is_image(video) {
        let (data, w, h) =
            crate::image::decode_scaled(video, Some((width, height)))?;
        crate::renderer::Pixels { data, width: w, height: h, stride: w * 4 }
    } else {
        let placement = Placement { mode: FitMode::Stretch, ..Default::default() };
        poster::render_first_frame(gpu, manager, video, (width, height), placement)?
    };

    let name = video
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "thumb".into());

    let path = directory.join(format!("{name}.jpg"));
    poster::write_jpeg(&pixels, &path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_library_loads_without_favorites_and_search_preserves_indices() {
        let mut library: Library = serde_json::from_value(serde_json::json!({"items":[
            {"name":"Forest","prepared":"forest.mp4","thumbnail":"forest.jpg","width":1,"height":1,"fps":30},
            {"name":"City","prepared":"city.mp4","thumbnail":"city.jpg","width":1,"height":1,"fps":30}
        ]})).unwrap();
        assert_eq!(library.matching(" FOREST ", false), [0]);
        assert!(library.matching("", true).is_empty());
        library.toggle_favorite(1);
        assert_eq!(library.matching("", true), [1]);
        let saved = serde_json::to_string(&library).unwrap();
        let restored: Library = serde_json::from_str(&saved).unwrap();
        assert_eq!(restored.matching("city", true), [1]);
    }
}
