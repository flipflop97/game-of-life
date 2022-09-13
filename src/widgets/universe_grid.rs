use crate::models::{Universe, UniverseGridMode, UniversePointMatrix, UniverseSnapshot};
use adw::prelude::AdwApplicationExt;
use gtk::{
    gio, glib,
    glib::{clone, Receiver, Sender},
    prelude::*,
    subclass::prelude::*,
    CompositeTemplate,
};

use std::cell::{Cell, RefCell};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

const FG_COLOR_LIGHT: &str = "#64baff";
const BG_COLOR_LIGHT: &str = "#fafafa";
const FG_COLOR_DARK: &str = "#C061CB";
const BG_COLOR_DARK: &str = "#3D3846";

#[derive(Debug)]
pub enum UniverseGridRequest {
    Freeze,
    Unfreeze,
    Mode(UniverseGridMode),
    DarkColorSchemePreference(bool),
    Run,
    Halt,
    Redraw,
}

mod imp {
    use super::*;
    use glib::{
        types::StaticType, ParamFlags, ParamSpec, ParamSpecBoolean, ParamSpecEnum, ParamSpecObject,
    };
    use once_cell::sync::Lazy;

    #[derive(Debug, Default, CompositeTemplate)]
    #[template(resource = "/com/github/sixpounder/GameOfLife/universe_grid.ui")]
    pub struct GameOfLifeUniverseGrid {
        #[template_child]
        pub drawing_area: TemplateChild<gtk::DrawingArea>,

        pub(crate) application: RefCell<Option<crate::GameOfLifeApplication>>,

        pub(crate) mode: Cell<UniverseGridMode>,

        pub(crate) frozen: Cell<bool>,

        pub(crate) prefers_dark_mode: Cell<bool>,

        pub(crate) universe: Arc<Mutex<Universe>>,

        pub(crate) receiver: RefCell<Option<Receiver<UniverseGridRequest>>>,

        pub(crate) sender: Option<Sender<UniverseGridRequest>>,

        pub(crate) render_thread_stopper: RefCell<Option<std::sync::mpsc::Receiver<()>>>,

        pub(crate) fg_color: std::cell::Cell<Option<gtk::gdk::RGBA>>,

        pub(crate) bg_color: std::cell::Cell<Option<gtk::gdk::RGBA>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for GameOfLifeUniverseGrid {
        const NAME: &'static str = "GameOfLifeUniverseGrid";
        type Type = super::GameOfLifeUniverseGrid;
        type ParentType = gtk::Widget;

        fn new() -> Self {
            let (sender, r) = glib::MainContext::channel(glib::PRIORITY_DEFAULT);
            let receiver = RefCell::new(Some(r));

            let mut this = Self::default();

            this.receiver = receiver;
            this.sender = Some(sender);
            this.mode.set(UniverseGridMode::Run);

            // Defaults to light color scheme
            this.fg_color
                .set(Some(gtk::gdk::RGBA::from_str(FG_COLOR_LIGHT).unwrap()));
            this.bg_color
                .set(Some(gtk::gdk::RGBA::from_str(BG_COLOR_DARK).unwrap()));

            this
        }

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
            klass.set_layout_manager_type::<gtk::BinLayout>();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for GameOfLifeUniverseGrid {
        fn constructed(&self, obj: &Self::Type) {
            self.parent_constructed(obj);

            obj.setup_drawing_area();
            obj.setup_channel();
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: Lazy<Vec<ParamSpec>> = Lazy::new(|| {
                vec![
                    ParamSpecObject::new(
                        "application",
                        "",
                        "",
                        crate::GameOfLifeApplication::static_type(),
                        ParamFlags::WRITABLE,
                    ),
                    ParamSpecEnum::new(
                        "mode",
                        "",
                        "",
                        UniverseGridMode::static_type(),
                        1,
                        ParamFlags::READWRITE,
                    ),
                    ParamSpecBoolean::new("frozen", "", "", false, ParamFlags::READWRITE),
                    ParamSpecBoolean::new("is-running", "", "", false, ParamFlags::READABLE),
                    ParamSpecBoolean::new(
                        "prefers-dark-mode",
                        "",
                        "",
                        false,
                        ParamFlags::READWRITE,
                    ),
                ]
            });
            PROPERTIES.as_ref()
        }

        fn set_property(
            &self,
            obj: &Self::Type,
            _id: usize,
            value: &glib::Value,
            pspec: &ParamSpec,
        ) {
            match pspec.name() {
                "mode" => {
                    obj.set_mode(value.get::<UniverseGridMode>().unwrap());
                }
                "frozen" => {
                    obj.set_frozen(value.get::<bool>().unwrap());
                }
                "application" => {
                    obj.imp()
                        .application
                        .replace(Some(value.get::<crate::GameOfLifeApplication>().unwrap()));
                }
                "prefers-dark-mode" => {
                    obj.imp()
                        .prefers_dark_mode
                        .replace(value.get::<bool>().unwrap());
                }
                _ => unimplemented!(),
            }
        }

        fn property(&self, obj: &Self::Type, _id: usize, pspec: &ParamSpec) -> glib::Value {
            match pspec.name() {
                "mode" => self.mode.get().to_value(),
                "frozen" => self.frozen.get().to_value(),
                "prefers-dark-mode" => self.prefers_dark_mode.get().to_value(),
                "is-running" => obj.is_running().to_value(),
                _ => unimplemented!(),
            }
        }
    }
    impl WidgetImpl for GameOfLifeUniverseGrid {}
}

glib::wrapper! {
    pub struct GameOfLifeUniverseGrid(ObjectSubclass<imp::GameOfLifeUniverseGrid>)
        @extends gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl GameOfLifeUniverseGrid {
    pub fn new<P: glib::IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::new(&[("application", application)])
            .expect("Failed to create GameOfLifeUniverseGrid")
    }

    fn setup_channel(&self) {
        let receiver = self.imp().receiver.borrow_mut().take().unwrap();
        receiver.attach(
            None,
            clone!(@strong self as this => move |action| this.process_action(action)),
        );
    }

    fn setup_drawing_area(&self) {
        if let Ok(application_ref) = self.try_property::<gtk::Application>("application") {
            if let Ok(application_ref) = application_ref.downcast::<adw::Application>() {
                application_ref.style_manager().connect_dark_notify(
                    clone!(@strong self as this => move |app| {
                        this.imp().prefers_dark_mode.set(app.is_dark());
                    }),
                );
            }
        }
        self.imp().drawing_area.connect_resize(
            clone!(@strong self as this => move |_widget, _width, _height| {
                this.set_frozen(true);
                let sender = this.get_sender();
                glib::timeout_add_once(std::time::Duration::from_millis(500), move || {
                    sender.send(UniverseGridRequest::Unfreeze).expect("Could not unlock grid");
                });
            }),
        );

        self.imp().drawing_area.set_draw_func(
            clone!(@strong self as this => move |widget, context, width, height| this.render(widget, context, width, height) ),
        );
    }

    fn process_action(&self, action: UniverseGridRequest) -> glib::Continue {
        match action {
            UniverseGridRequest::Freeze => self.set_frozen(true),
            UniverseGridRequest::Unfreeze => self.set_frozen(false),
            UniverseGridRequest::Mode(m) => self.set_mode(m),
            UniverseGridRequest::Run => self.run(),
            UniverseGridRequest::Halt => self.halt(),
            UniverseGridRequest::Redraw => self.imp().drawing_area.queue_draw(),
            UniverseGridRequest::DarkColorSchemePreference(prefers_dark) => {
                self.set_prefers_dark_mode(prefers_dark)
            }
        }

        glib::Continue(true)
    }

    pub fn set_prefers_dark_mode(&self, prefers_dark_variant: bool) {
        let imp = self.imp();
        imp.prefers_dark_mode.replace(prefers_dark_variant);

        match prefers_dark_variant {
            true => {
                imp.fg_color
                    .set(Some(gtk::gdk::RGBA::from_str(FG_COLOR_DARK).unwrap()));
                imp.bg_color
                    .set(Some(gtk::gdk::RGBA::from_str(BG_COLOR_DARK).unwrap()));
            }
            false => {
                imp.fg_color
                    .set(Some(gtk::gdk::RGBA::from_str(FG_COLOR_LIGHT).unwrap()));
                imp.bg_color
                    .set(Some(gtk::gdk::RGBA::from_str(BG_COLOR_LIGHT).unwrap()));
            }
        }
    }

    pub fn prefers_dark_mode(&self) -> bool {
        self.imp().prefers_dark_mode.get()
    }

    fn render(
        &self,
        _widget: &gtk::DrawingArea,
        context: &gtk::cairo::Context,
        width: i32,
        height: i32,
    ) {
        if !self.frozen() {
            let imp = self.imp();
            let fg_color = imp.fg_color.get().unwrap();
            let bg_color = imp.bg_color.get().unwrap();
            let universe = self.imp().universe.lock().unwrap();

            context.set_source_rgba(
                bg_color.red() as f64,
                bg_color.green() as f64,
                bg_color.blue() as f64,
                bg_color.alpha() as f64,
            );
            context.rectangle(0.0, 0.0, width.into(), height.into());
            context.fill().unwrap();

            let mut size: (f64, f64) = (
                width as f64 / universe.columns() as f64,
                height as f64 / universe.rows() as f64,
            );

            if size.0 <= size.1 {
                size = (size.0, size.0);
            } else {
                size = (size.1, size.1);
            }

            context.set_source_rgba(
                fg_color.red() as f64,
                fg_color.green() as f64,
                fg_color.blue() as f64,
                fg_color.alpha() as f64,
            );

            for el in universe.iter_cells() {
                if el.cell().is_alive() {
                    let w = el.row();
                    let h = el.column();
                    let coords: (f64, f64) = ((w as f64) * size.0, (h as f64) * size.1);

                    context.rectangle(coords.0, coords.1, size.0, size.1);
                    context.fill().unwrap();
                }
            }
        }
    }

    pub fn mode(&self) -> UniverseGridMode {
        self.imp().mode.get()
    }

    pub fn set_mode(&self, value: UniverseGridMode) {
        if !self.is_running() {
            self.imp().mode.set(value);

            match self.mode() {
                UniverseGridMode::Design => {}
                UniverseGridMode::Run => {}
            }
        }

        self.notify("mode");
    }

    pub fn is_running(&self) -> bool {
        self.imp().render_thread_stopper.borrow().is_some()
    }

    pub fn set_frozen(&self, value: bool) {
        match value {
            false => {
                self.imp().drawing_area.queue_draw();
            }
            _ => (),
        }

        self.imp().frozen.set(value);
    }

    pub fn frozen(&self) -> bool {
        self.imp().frozen.get()
    }

    pub fn get_sender(&self) -> Sender<UniverseGridRequest> {
        self.imp().sender.as_ref().unwrap().clone()
    }

    pub fn run(&self) {
        self.set_mode(UniverseGridMode::Run);

        let universe = self.imp().universe.clone();
        let local_sender = self.get_sender();

        let (thread_render_stopper_sender, thread_render_stopper_receiver) =
            std::sync::mpsc::channel::<()>();

        // Drop this to stop ticking thread
        self.imp()
            .render_thread_stopper
            .replace(Some(thread_render_stopper_receiver));

        std::thread::spawn(move || loop {
            match thread_render_stopper_sender.send(()) {
                Ok(_) => (),
                Err(_) => break,
            };

            std::thread::sleep(std::time::Duration::from_millis(50));
            let mut locked_universe = universe.lock().unwrap();
            locked_universe.tick();
            local_sender.send(UniverseGridRequest::Redraw).unwrap();
        });

        self.notify("is-running");
    }

    pub fn halt(&self) {
        let inner = self.imp().render_thread_stopper.take();
        drop(inner);
        self.notify("is-running");
    }

    pub fn get_universe_snapshot(&self) -> UniverseSnapshot {
        let imp = self.imp();

        let clone = Arc::clone(&imp.universe);
        let lock = clone.lock().unwrap();

        lock.snapshot()
    }

    pub fn random_seed(&self) {
        let mut lock = self.imp().universe.lock().unwrap();
        let (rows, cols) = (lock.rows(), lock.columns());
        *lock = Universe::new_random(rows, cols);
        self.process_action(UniverseGridRequest::Redraw);
    }
}


