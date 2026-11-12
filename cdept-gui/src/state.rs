//! State shared by all views of the main window.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use libcdept::tools::{self, ToolInfo};
use serde::{Deserialize, Serialize};

use crate::dock::Tab;
use crate::log::LogBuffer;

/// User settings persisted across sessions (the dock layout is stored separately).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub recent_projects: Vec<PathBuf>,
    pub console_level: ConsoleLevel,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            recent_projects: Vec::new(),
            console_level: ConsoleLevel::Info,
        }
    }
}

const MAX_RECENT: usize = 10;

impl Settings {
    pub fn add_recent(&mut self, path: PathBuf) {
        self.recent_projects.retain(|p| p != &path);
        self.recent_projects.insert(0, path);
        self.recent_projects.truncate(MAX_RECENT);
    }
}

/// Minimum level shown in the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConsoleLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl ConsoleLevel {
    pub const ALL: [ConsoleLevel; 5] = [
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    pub fn shows(self, level: tracing::Level) -> bool {
        let max = match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        };
        level <= max
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
    }
}

/// Requests from views to the shell, applied after the frame's UI is built.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    OpenTab(Tab),
    ResetLayout,
    DetectTools,
    ShowAbout,
    Quit,
}

/// External tool detection, run on a background thread.
#[derive(Default)]
pub struct ToolsState {
    result: Arc<Mutex<Option<Vec<ToolInfo>>>>,
    running: Arc<Mutex<bool>>,
}

impl ToolsState {
    pub fn start_detection(&self, ctx: &egui::Context) {
        {
            let mut running = self.running.lock().unwrap();
            if *running {
                return;
            }
            *running = true;
        }
        let (result, running, ctx) = (self.result.clone(), self.running.clone(), ctx.clone());
        thread::spawn(move || {
            let tools = tools::detect_all();
            let found = tools.iter().filter(|t| t.found()).count();
            tracing::info!(target: "cdept::tools", "found {found} of {} external tools", tools.len());
            for t in tools.iter().filter(|t| !t.found()) {
                tracing::debug!(target: "cdept::tools", "{} not found in PATH", t.spec.name);
            }
            *result.lock().unwrap() = Some(tools);
            *running.lock().unwrap() = false;
            ctx.request_repaint();
        });
    }

    pub fn is_running(&self) -> bool {
        *self.running.lock().unwrap()
    }

    /// Snapshot of the last detection, if any.
    pub fn tools(&self) -> Option<Vec<ToolInfo>> {
        self.result.lock().unwrap().clone()
    }

    pub fn find(&self, name: &str) -> Option<ToolInfo> {
        self.tools()?.into_iter().find(|t| t.spec.name == name)
    }
}

/// Everything views can read or change, other than the dock layout itself.
pub struct AppCtx {
    pub settings: Settings,
    pub log: LogBuffer,
    pub tools: ToolsState,
    pub project: Option<PathBuf>,
    actions: Vec<Action>,
}

impl AppCtx {
    pub fn new(settings: Settings, log: LogBuffer) -> Self {
        Self {
            settings,
            log,
            tools: ToolsState::default(),
            project: None,
            actions: Vec::new(),
        }
    }

    pub fn request(&mut self, action: Action) {
        if !self.actions.contains(&action) {
            self.actions.push(action);
        }
    }

    pub(crate) fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.actions)
    }

    /// The `$PDK` environment variable, as set by open_pdks / IIC-OSIC-TOOLS.
    pub fn pdk(&self) -> Option<String> {
        std::env::var("PDK").ok().filter(|s| !s.is_empty())
    }
}
