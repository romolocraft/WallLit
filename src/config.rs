use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use windows::core::Result;
use windows::Win32::UI::Shell::{FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_CREATE};

use crate::language::Language;
use crate::renderer::{Area, FitMode, Look, Placement};

const CURRENT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Fill,
    Fit,
    Stretch,
    Center,
    Custom,
}

impl From<Mode> for FitMode {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Fill => FitMode::Fill,
            Mode::Fit => FitMode::Fit,
            Mode::Stretch => FitMode::Stretch,
            Mode::Center => FitMode::Center,
            Mode::Custom => FitMode::Custom,
        }
    }
}

impl From<FitMode> for Mode {
    fn from(m: FitMode) -> Self {
        match m {
            FitMode::Fill => Mode::Fill,
            FitMode::Fit => Mode::Fit,
            FitMode::Stretch => Mode::Stretch,
            FitMode::Center => Mode::Center,
            FitMode::Custom => Mode::Custom,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(tag = "unit", content = "value", rename_all = "lowercase")]
pub enum Hold {
    Loops(u32),

    Seconds(f32),
}

impl Default for Hold {
    fn default() -> Self {
        Hold::Loops(3)
    }
}

impl Hold {
    pub const IMAGE_DEFAULT: Hold = Hold::Seconds(30.0);

    pub fn duration_100ns(&self, media_duration_100ns: i64) -> i64 {
        match self {
            Hold::Loops(times) => {
                let once = if media_duration_100ns > 0 {
                    media_duration_100ns
                } else {
                    600_000_000
                };
                once.saturating_mul((*times).max(1) as i64)
            }
            Hold::Seconds(seconds) => (seconds.max(1.0) as i64).saturating_mul(10_000_000),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Transition {
    #[default]
    None,
    Fade,
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    Rebuild,
    Morph,
}

impl Transition {
    pub const ALL: [Transition; 8] = [
        Transition::None,
        Transition::Fade,
        Transition::LeftToRight,
        Transition::RightToLeft,
        Transition::TopToBottom,
        Transition::BottomToTop,
        Transition::Rebuild,
        Transition::Morph,
    ];
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleEntry {
    pub minute: u16,

    pub slide: usize,
}

impl ScheduleEntry {
    pub fn label(&self) -> String {
        format!("{:02}:{:02}", self.minute / 60, self.minute % 60)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Schedule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub entries: Vec<ScheduleEntry>,
}

impl Schedule {
    pub fn day_and_night() -> Vec<ScheduleEntry> {
        vec![
            ScheduleEntry { minute: 7 * 60, slide: 0 },
            ScheduleEntry { minute: 19 * 60, slide: 1 },
        ]
    }

    pub fn active_at(&self, minute: u16) -> Option<(usize, u16)> {
        if !self.enabled || self.entries.is_empty() {
            return None;
        }

        let mut ordered = self.entries.clone();
        ordered.sort_by_key(|entry| entry.minute);

        let position = ordered
            .iter()
            .rposition(|entry| entry.minute <= minute)
            .unwrap_or(ordered.len() - 1);

        let next = ordered[(position + 1) % ordered.len()].minute;
        Some((ordered[position].slide, next))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Filters {
    #[serde(default)]
    pub brightness: f32,
    #[serde(default = "default_one")]
    pub contrast: f32,
    #[serde(default = "default_one")]
    pub saturation: f32,
    #[serde(default)]
    pub temperature: f32,
}

impl Default for Filters {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 1.0, saturation: 1.0, temperature: 0.0 }
    }
}

impl Filters {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    pub fn look(&self) -> Look {
        Look {
            brightness: self.brightness,
            contrast: self.contrast,
            saturation: self.saturation,
            temperature: self.temperature,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Layer {
    pub wallpaper: PathBuf,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,

    #[serde(default = "default_rect")]
    pub rect: [f32; 4],

    #[serde(default = "default_layer_mode")]
    pub mode: Mode,

    #[serde(default = "default_scale")]
    pub scale: f32,

    #[serde(default)]
    pub x: f32,

    #[serde(default)]
    pub y: f32,

    #[serde(default = "default_one")]
    pub stretch_x: f32,

    #[serde(default = "default_one")]
    pub stretch_y: f32,

    #[serde(default = "default_speed")]
    pub speed: f32,

    #[serde(default)]
    pub filters: Filters,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            wallpaper: PathBuf::new(),
            source: None,
            rect: default_rect(),
            mode: default_layer_mode(),
            scale: default_scale(),
            x: 0.0,
            y: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            speed: default_speed(),
            filters: Filters::default(),
        }
    }
}

impl Layer {
    pub fn display_name(&self) -> String {
        self.source
            .as_deref()
            .unwrap_or(&self.wallpaper)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn placement(&self) -> Placement {
        Placement {
            mode: self.mode.into(),
            scale: self.scale,
            offset_x: self.x,
            offset_y: self.y,
            stretch_x: self.stretch_x,
            stretch_y: self.stretch_y,
            look: self.filters.look(),
        }
    }

    pub fn area(&self, size: (u32, u32)) -> Area {
        let [x, y, w, h] = self.rect;
        Area {
            x: x * size.0 as f32,
            y: y * size.1 as f32,
            w: (w * size.0 as f32).max(1.0),
            h: (h * size.1 as f32).max(1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Slide {
    pub wallpaper: PathBuf,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PathBuf>,

    #[serde(default = "default_mode")]
    pub mode: Mode,

    #[serde(default = "default_scale")]
    pub scale: f32,

    #[serde(default)]
    pub x: f32,

    #[serde(default)]
    pub y: f32,

    #[serde(default = "default_one")]
    pub stretch_x: f32,

    #[serde(default = "default_one")]
    pub stretch_y: f32,

    #[serde(default = "default_speed")]
    pub speed: f32,

    #[serde(default)]
    pub filters: Filters,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<Hold>,
}

impl Default for Slide {
    fn default() -> Self {
        Self {
            wallpaper: PathBuf::new(),
            source: None,
            mode: default_mode(),
            scale: default_scale(),
            x: 0.0,
            y: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            speed: default_speed(),
            filters: Filters::default(),
            layers: Vec::new(),
            hold: None,
        }
    }
}

impl Slide {
    pub fn display_name(&self) -> String {
        self.source
            .as_deref()
            .unwrap_or(&self.wallpaper)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn placement(&self) -> Placement {
        Placement {
            mode: self.mode.into(),
            scale: self.scale,
            offset_x: self.x,
            offset_y: self.y,
            stretch_x: self.stretch_x,
            stretch_y: self.stretch_y,
            look: self.filters.look(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MonitorConfig {
    #[serde(default)]
    pub slides: Vec<Slide>,

    #[serde(default)]
    pub transition: Transition,

    #[serde(default)]
    pub hold: Hold,

    #[serde(default = "default_true")]
    pub uniform_hold: bool,

    #[serde(default)]
    pub schedule: Schedule,

    #[serde(default, skip_serializing)]
    wallpaper: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    source: Option<PathBuf>,
    #[serde(default, skip_serializing)]
    mode: Option<Mode>,
    #[serde(default, skip_serializing)]
    scale: Option<f32>,
    #[serde(default, skip_serializing)]
    x: Option<f32>,
    #[serde(default, skip_serializing)]
    y: Option<f32>,
    #[serde(default, skip_serializing)]
    speed: Option<f32>,
}

impl MonitorConfig {
    fn migrate(&mut self) {
        let Some(wallpaper) = self.wallpaper.take() else { return };
        if !self.slides.is_empty() || wallpaper.as_os_str().is_empty() {
            return;
        }

        self.slides.push(Slide {
            wallpaper,
            source: self.source.take(),
            mode: self.mode.take().unwrap_or_else(default_mode),
            scale: self.scale.take().unwrap_or_else(default_scale),
            x: self.x.take().unwrap_or(0.0),
            y: self.y.take().unwrap_or(0.0),
            speed: self.speed.take().unwrap_or_else(default_speed),
            hold: None,
            ..Default::default()
        });
    }

    pub fn from_slides(slides: Vec<Slide>) -> Self {
        Self { slides, ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.slides.is_empty()
    }

    pub fn hold_for(&self, slide: &Slide) -> Hold {
        if self.uniform_hold {
            self.hold
        } else {
            slide.hold.unwrap_or(self.hold)
        }
    }
}

fn default_one() -> f32 {
    1.0
}

fn default_rect() -> [f32; 4] {
    [0.25, 0.25, 0.5, 0.5]
}

fn default_layer_mode() -> Mode {
    Mode::Fill
}

fn default_mode() -> Mode {
    Mode::Fill
}

fn default_scale() -> f32 {
    1.0
}

fn default_speed() -> f32 {
    1.0
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,

    #[serde(default)]
    pub monitors: BTreeMap<String, MonitorConfig>,

    #[serde(default = "default_true")]
    pub pause_when_fullscreen: bool,

    #[serde(default)]
    pub pause_on_battery: bool,

    #[serde(default = "default_true")]
    pub pause_on_energy_saver: bool,

    #[serde(default = "default_true")]
    pub optimize_wallpaper: bool,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<Language>,

    #[serde(default = "default_true")]
    pub close_to_tray: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            monitors: BTreeMap::new(),
            pause_when_fullscreen: true,
            pause_on_battery: false,
            pause_on_energy_saver: true,
            optimize_wallpaper: true,
            close_to_tray: true,
            language: None,
        }
    }
}

impl Config {
    pub fn language(&self) -> Language {
        self.language.unwrap_or_else(Language::from_system)
    }

    pub fn load() -> Self {
        match Self::try_load() {
            Ok(config) => config,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                eprintln!("could not read the settings: {e}");
                Self::default()
            }
        }
    }

    pub fn try_load() -> std::io::Result<Self> {
        let path = config_path().map_err(|e| std::io::Error::other(e.message()))?;
        Self::parse(&std::fs::read_to_string(path)?)
    }

    pub fn read_or_default() -> std::io::Result<Self> {
        match Self::try_load() {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            other => other,
        }
    }

    pub fn parse(text: &str) -> std::io::Result<Self> {
        let mut config: Self = serde_json::from_str(without_bom(text))?;
        for monitor in config.monitors.values_mut() {
            monitor.migrate();
        }
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> std::io::Result<()> {
        let invalid = |message| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
        if self.version != CURRENT_VERSION {
            return Err(invalid("unsupported settings version"));
        }
        let valid_hold = |hold: Hold| match hold {
            Hold::Loops(n) => n > 0,
            Hold::Seconds(n) => n.is_finite() && n >= 1.0,
        };
        for monitor in self.monitors.values() {
            if !valid_hold(monitor.hold) {
                return Err(invalid("invalid display time"));
            }
            for slide in &monitor.slides {
                if slide.wallpaper.as_os_str().is_empty()
                    || !slide.speed.is_finite() || slide.speed <= 0.0
                    || !slide.scale.is_finite() || slide.scale <= 0.0
                    || !slide.x.is_finite() || !slide.y.is_finite()
                    || slide.hold.is_some_and(|h| !valid_hold(h))
                {
                    return Err(invalid("wallpaper with invalid path, framing or timing"));
                }
            }
            for entry in &monitor.schedule.entries {
                if entry.minute >= 1440 {
                    return Err(invalid("horario de agendamento fora do dia"));
                }
            }
        }
        Ok(())
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.validate()?;
        let path = config_path()
            .map_err(|e| std::io::Error::other(format!("settings folder: {e}")))?;
        crate::storage::write_json(&path, self)
    }
}

pub fn without_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

pub fn data_dir() -> Result<PathBuf> {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("WALLLIT_TEST_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_CREATE, None) }?;
    let base = PathBuf::from(unsafe { raw.to_string() }.unwrap_or_default());
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as *const _)) };
    Ok(base.join("WallLit"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("config.json"))
}

pub fn media_dir() -> Result<PathBuf> {
    Ok(data_dir()?.join("wallpapers"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_bom_and_migrates_legacy_wallpaper() {
        let config = Config::parse("\u{feff}{\"version\":1,\"monitors\":{\"screen\":{\"wallpaper\":\"old.mp4\",\"speed\":0.5}}}").unwrap();
        let slides = &config.monitors["screen"].slides;
        assert_eq!(slides.len(), 1);
        assert_eq!(slides[0].wallpaper, PathBuf::from("old.mp4"));
        assert_eq!(slides[0].speed, 0.5);
    }

    #[test]
    fn invalid_json_and_future_version_are_errors_not_empty_configurations() {
        assert!(Config::parse("{").is_err());
        assert!(Config::parse(r#"{"version":2}"#).is_err());
        assert!(Config::parse(r#"{"version":1,"monitors":{}}"#).unwrap().monitors.is_empty());
    }

    #[test]
    fn rejects_invalid_playback_parameters() {
        for field in [r#""speed":0"#, r#""speed":-1"#, r#""scale":0"#,
            r#""hold":{"unit":"seconds","value":0}"#] {
            let text = format!(r#"{{"version":1,"monitors":{{"screen":{{"slides":[{{"wallpaper":"a.mp4",{field}}}]}}}}}}"#);
            assert!(Config::parse(&text).is_err(), "{field}");
        }
    }

    #[test]
    fn schedule_wraps_midnight_and_rejects_out_of_day_time() {
        let schedule = Schedule { enabled: true, entries: Schedule::day_and_night() };
        assert_eq!(schedule.active_at(0), Some((1, 420)));
        assert_eq!(schedule.active_at(420), Some((0, 1140)));
        assert_eq!(schedule.active_at(1140), Some((1, 420)));
        assert!(Config::parse(r#"{"version":1,"monitors":{"screen":{"schedule":{"entries":[{"minute":1440,"slide":0}]}}}}"#).is_err());
    }
}
