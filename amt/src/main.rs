
mod main_impl {
    use egui::Vec2;
    use eyre::Result;
    use libamt::{
        run_egui,
        LibraryHandlerGUI,
    };

    pub(crate) fn main() -> Result<()> {
        let main_app = LibraryHandlerGUI::default();

        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_app_id("org.ckdur.amt")
                .with_title("Analog Management Tool")
                .with_inner_size(Vec2::new(
                    1920 as f32, // TODO: Load this from configurations
                    1080 as f32,
                )),
            ..Default::default()
        };

        eframe::run_native(
            "Analog Management Tool",
            options,
            Box::new(|_cc| Ok(run_egui(_cc, main_app)?)),
        ).unwrap();

        Ok(())
    }
}

fn main() -> eyre::Result<()> {
    main_impl::main()
}

