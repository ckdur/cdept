
mod main_impl {
    use egui::Vec2;
    use eyre::Result;
    use libcdept::{
        run_egui,
        LibraryHandlerGUI,
    };

    pub(crate) fn main() -> Result<()> {
        let main_app = LibraryHandlerGUI::default();

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("org.ckdur.cdept")
                .with_title("Chip Design Project Tool (CDePT)")
                .with_inner_size(Vec2::new(
                    1920 as f32, // TODO: Load this from configurations
                    1080 as f32,
                )),
            ..Default::default()
        };

        eframe::run_native(
            "Chip Design Project Tool (CDePT)",
            options,
            Box::new(|_cc| Ok(run_egui(_cc, main_app)?)),
        ).unwrap();

        Ok(())
    }
}

fn main() -> eyre::Result<()> {
    main_impl::main()
}

