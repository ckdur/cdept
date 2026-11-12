pub mod library_handler_gui;

pub use library_handler_gui::LibraryHandlerGUI;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, RwLock};

use egui::{FontData, FontDefinitions, FontFamily};
use eframe::{App, CreationContext};
use eyre::Result;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref EGUI_CONTEXT: RwLock<Option<Arc<egui::Context>>> = RwLock::new(None);
    /// A number that is non-zero if there are asynchronously triggered operations that
    /// have been triggered but not successfully completed yet. In practice, if this is
    /// non-zero, we will re-run the egui update function in order to ensure that we deal
    /// with the outstanding transactions eventually.
    /// When incrementing this, it is important to make sure that it gets decremented
    /// whenever the asynchronous transaction is completed, otherwise we will re-render
    /// things until program exit
    pub(crate) static ref OUTSTANDING_TRANSACTIONS: AtomicU32 = AtomicU32::new(0);
}

pub fn run_egui(cc: &CreationContext, mut main_app: LibraryHandlerGUI) -> Result<Box<dyn App>> {
    let ctx_arc = Arc::new(cc.egui_ctx.clone());
    *EGUI_CONTEXT.write().unwrap() = Some(ctx_arc.clone());

    // TODO: If change of visuals, we need the context
    //main_app.context = Some(ctx_arc.clone());

    // TODO: Change the visuals according to theme
    //cc.egui_ctx.set_visuals_of(egui::Theme::Dark, main_app.get_visuals());
    //cc.egui_ctx.set_visuals_of(egui::Theme::Light, main_app.get_visuals());

    // TODO: Change the font if requested
    //setup_custom_font(&cc.egui_ctx); 
    Ok(Box::new(main_app))
}
