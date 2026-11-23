//! The Rust side of the bridge (ARCHITECTURE.md A10): what the bridge's ActionScript (`as3/`) calls
//! through `ExternalInterface`, Scaleform's translator, the movie's log and a navigator that loads
//! nothing. Every ActionScript call arrives in `Interface::call_method` by the name of Elite's native
//! method; questions go to the `Host`, notices become `Out`s.

use crate::easing;
use crate::host::Host;
use crate::movie::Out;
use elite_ui::Value;
use ruffle_core::backend::log::LogBackend;
use ruffle_core::backend::navigator::{
    ErrorResponse, NavigationMethod, NavigatorBackend, NullExecutor, NullSpawner, OwnedFuture, Request,
    SuccessResponse, async_return, create_fetch_error,
};
use ruffle_core::backend::ui::{
    DialogResultFuture, FileFilter, FontDefinition, FullscreenError, LanguageIdentifier, MouseCursor,
    MultiDialogResultFuture, NullUiBackend, UiBackend,
};
use ruffle_core::context::UpdateContext;
use ruffle_core::external::{ExternalInterfaceProvider, Value as Ext};
use ruffle_core::font::FontQuery;
use ruffle_core::indexmap::IndexMap;
use ruffle_core::loader::Error as LoadError;
use ruffle_core::socket::{ConnectionState, SocketAction, SocketHandle};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use url::{ParseError, Url};

/// The bridge's callback that installs Elite's native interfaces (`as3/elitegfx/Bridge.as`).
pub const BIND: &str = "elitegfx.bind";
/// The bridge's callback for the game's messages (registered when the bridge starts).
pub const HANDLE_MESSAGE: &str = "HandleMessage";
/// The bridge's callback that writes the DataStore.
pub const STORE: &str = "elitegfx.store";

/// What one movie's providers share with its `Movie`.
pub struct Link {
    pub movie: String,
    pub host: Arc<dyn Host>,
    /// The movie's frame rate: the tweener's clock steps by one frame of it.
    pub frame_rate: f64,
    pub outbox: RefCell<Vec<Out>>,
    /// The movie called `SetReadyForInvokes`.
    pub ready: Cell<bool>,
    /// The bridge's callbacks are registered (its script has run).
    pub bridge_up: Cell<bool>,
}

impl Link {
    fn post(&self, out: Out) {
        self.outbox.borrow_mut().push(out);
    }
}

pub fn to_ext(value: &Value) -> Ext {
    match value {
        Value::Null => Ext::Null,
        Value::Bool(b) => Ext::Bool(*b),
        Value::Num(n) => Ext::Number(*n),
        Value::Str(s) => Ext::String(s.clone()),
        Value::Array(items) => Ext::List(items.iter().map(to_ext).collect()),
        // Ruffle's interchange object is a sorted map: ActionScript reads fields by name.
        Value::Object(fields) => {
            Ext::Object(fields.iter().map(|(k, v)| (k.clone(), to_ext(v))).collect::<BTreeMap<_, _>>())
        }
    }
}

pub fn from_ext(value: &Ext) -> Value {
    match value {
        Ext::Undefined | Ext::Null => Value::Null,
        Ext::Bool(b) => Value::Bool(*b),
        Ext::Number(n) => Value::Num(*n),
        Ext::String(s) => Value::Str(s.clone()),
        Ext::List(items) => Value::Array(items.iter().map(from_ext).collect()),
        Ext::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), from_ext(v))).collect()),
    }
}

/// Argument `i` as text (absent or null: empty).
fn text(args: &[Ext], i: usize) -> String {
    match args.get(i) {
        Some(Ext::String(s)) => s.clone(),
        Some(Ext::Number(n)) => n.to_string(),
        Some(Ext::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

/// Argument `i` as text, or `None` if absent or null.
fn optional_text(args: &[Ext], i: usize) -> Option<String> {
    matches!(args.get(i), Some(Ext::String(_) | Ext::Number(_) | Ext::Bool(_))).then(|| text(args, i))
}

fn number(args: &[Ext], i: usize) -> f64 {
    match args.get(i) {
        Some(Ext::Number(n)) => *n,
        Some(Ext::Bool(b)) => f64::from(u8::from(*b)),
        Some(Ext::String(s)) => s.trim().parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// Argument `i` as an array of values (absent or null: empty).
fn list(args: &[Ext], i: usize) -> Vec<Value> {
    match args.get(i).map(from_ext) {
        Some(Value::Array(items)) => items,
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![other],
    }
}

fn show(args: &[Ext]) -> String {
    args.iter().map(|a| format!("{:?}", from_ext(a))).collect::<Vec<_>>().join(", ")
}

/// The movie's `ExternalInterface`: the bridge's calls by the names of Elite's native methods.
pub struct Interface(pub Rc<Link>);

impl ExternalInterfaceProvider for Interface {
    fn call_method(&self, context: &mut UpdateContext<'_>, name: &str, args: &[Ext]) -> Ext {
        let link = &self.0;
        let host = &link.host;
        match name {
            // Elite's movies ask the exe to fill in their factories' functions; the bridge does it
            // in ActionScript, so this hands the call to the bridge's callback.
            "BindDirectInterfaces" => match context.external_interface.get_callback(BIND) {
                Some(bind) => {
                    bind.call(context, BIND, [Ext::Number(link.frame_rate)]);
                }
                None => link.post(Out::Trace("BindDirectInterfaces: the bridge is not in this movie".into())),
            },
            "Ease" => return Ext::Number(easing::ease(number(args, 0) as i32, number(args, 1))),

            // Notices.
            "SetReadyForInvokes" => {
                link.ready.set(true);
                link.post(Out::Ready);
            }
            "UIOpened" => link.post(Out::Opened),
            "UIClosed" => link.post(Out::Closed),
            "SendEventToGame" => link.post(Out::Event { name: text(args, 0), args: list(args, 1) }),
            "DoAction" => link.post(Out::Action { name: text(args, 0), args: list(args, 1) }),
            // The movie asking for buttons, the mouse or reporting an input event: the game's input
            // routing decides, so they travel as actions under their own names.
            "CapturingButtons" | "CapturingMouse" | "OnInputEvent" => link.post(Out::Action {
                name: name.to_string(),
                args: args.iter().map(from_ext).collect(),
            }),
            "PlaySound" => link.post(Out::Sound { name: text(args, 0), source: optional_text(args, 1) }),
            "Trace" => link.post(Out::Trace(text(args, 0))),
            "Break" => link.post(Out::Trace(format!("Break: {}", text(args, 0)))),

            // Questions.
            "Translate" => return Ext::String(host.translate(&text(args, 0))),
            "GetString" => return Ext::String(host.get_string(&text(args, 0))),
            "MakeUntranslatedString" => return Ext::String(host.make_untranslated_string(&text(args, 0))),
            "FindInTranslationString" => {
                return Ext::Number(host.find_in_translation_string(&text(args, 0), &text(args, 1), number(args, 2)));
            }
            "LocaliseNumber" => {
                let extra = optional_text(args, 2);
                return Ext::String(host.localise_number(number(args, 0), &text(args, 1), extra.as_deref()));
            }
            "GetMovieName" => return Ext::String(host.get_movie_name(&link.movie)),
            "GetDatabaseItemData" => return to_ext(&host.get_database_item_data(number(args, 0) as u32)),
            "GetData" => return to_ext(&host.get_data(&text(args, 0), number(args, 1) as u32)),
            "DoesIconDataResourceExist" => return Ext::Bool(host.does_icon_data_resource_exist(&text(args, 0))),
            "DoesTextSymbolExist" => return Ext::Bool(host.does_text_symbol_exist(&text(args, 0))),
            "OptionsFlag" => return Ext::Bool(host.options_flag(&text(args, 0))),
            "GetMappedFontName" => return Ext::String(host.get_mapped_font_name(&text(args, 0))),
            _ => link.post(Out::Trace(format!("unknown bridge call {name}({})", show(args)))),
        }
        Ext::Undefined
    }

    fn on_callback_available(&self, name: &str) {
        if name == STORE {
            self.0.bridge_up.set(true);
        }
    }

    fn get_id(&self) -> Option<String> {
        None
    }
}

/// The movie's `trace` and the player's script warnings (an uncaught ActionScript error is traced:
/// the player runs in debug mode).
pub struct Log(pub Rc<Link>);

impl LogBackend for Log {
    fn avm_trace(&self, message: &str) {
        self.0.post(Out::Trace(message.to_string()));
    }

    fn avm_warning(&self, message: &str) {
        self.0.post(Out::Trace(format!("warning: {message}")));
    }
}

/// Scaleform's Translator on every text field (Ruffle patch a), and Ruffle's null UI otherwise.
pub struct Ui {
    link: Rc<Link>,
    null: NullUiBackend,
}

impl Ui {
    pub fn new(link: Rc<Link>) -> Self {
        Self { link, null: NullUiBackend::new() }
    }
}

impl UiBackend for Ui {
    fn translate_text(&self, text: &str) -> Option<String> {
        let translated = self.link.host.translate(text);
        (translated != text).then_some(translated)
    }

    fn mouse_visible(&self) -> bool {
        self.null.mouse_visible()
    }
    fn set_mouse_visible(&mut self, visible: bool) {
        self.null.set_mouse_visible(visible)
    }
    fn set_mouse_cursor(&mut self, cursor: MouseCursor) {
        self.null.set_mouse_cursor(cursor)
    }
    fn clipboard_content(&mut self) -> String {
        self.null.clipboard_content()
    }
    fn set_clipboard_content(&mut self, content: String) {
        self.null.set_clipboard_content(content)
    }
    fn set_fullscreen(&mut self, is_full: bool) -> Result<(), FullscreenError> {
        self.null.set_fullscreen(is_full)
    }
    fn display_root_movie_download_failed_message(&self, invalid_swf: bool, fetch_error: String) {
        self.null.display_root_movie_download_failed_message(invalid_swf, fetch_error)
    }
    fn message(&self, message: &str) {
        self.null.message(message)
    }
    fn open_virtual_keyboard(&self) {}
    fn close_virtual_keyboard(&self) {}
    fn language(&self) -> LanguageIdentifier {
        self.null.language()
    }
    fn display_unsupported_video(&self, url: Url) {
        self.null.display_unsupported_video(url)
    }
    fn load_device_font(&self, query: &FontQuery, register: &mut dyn FnMut(FontDefinition)) {
        self.null.load_device_font(query, register)
    }
    fn sort_device_fonts(&self, query: &FontQuery, register: &mut dyn FnMut(FontDefinition)) -> Vec<FontQuery> {
        self.null.sort_device_fonts(query, register)
    }
    fn display_file_open_dialog(&mut self, filters: Vec<FileFilter>) -> Option<DialogResultFuture> {
        self.null.display_file_open_dialog(filters)
    }
    fn display_file_open_dialog_multiple(&mut self, filters: Vec<FileFilter>) -> Option<MultiDialogResultFuture> {
        self.null.display_file_open_dialog_multiple(filters)
    }
    fn display_file_save_dialog(&mut self, file_name: String, title: String) -> Option<DialogResultFuture> {
        self.null.display_file_save_dialog(file_name, title)
    }
    fn close_file_dialog(&mut self) {
        self.null.close_file_dialog()
    }
}

/// A navigator that loads nothing: a movie's files are all in its converted SWF (the font
/// library it names is the game's registered fonts; images are not loaded).
pub struct NoFiles {
    spawner: NullSpawner,
}

impl NoFiles {
    pub fn new(executor: &NullExecutor) -> Self {
        Self { spawner: executor.spawner() }
    }
}

impl NavigatorBackend for NoFiles {
    fn navigate_to_url(&self, _url: &str, _target: &str, _vars: Option<(NavigationMethod, IndexMap<String, String>)>) {}

    fn fetch(&self, request: Request) -> OwnedFuture<Box<dyn SuccessResponse>, ErrorResponse> {
        async_return(Err(create_fetch_error(request.url(), "a movie loads no files")))
    }

    fn resolve_url(&self, url: &str) -> Result<Url, ParseError> {
        Url::parse(url).or_else(|_| Url::parse("file:///movie/")?.join(url))
    }

    fn spawn_future(&mut self, future: OwnedFuture<(), LoadError>) {
        self.spawner.spawn_local(future);
    }

    fn pre_process_url(&self, url: Url) -> Url {
        url
    }

    fn connect_socket(
        &mut self,
        _host: String,
        _port: u16,
        _timeout: Duration,
        handle: SocketHandle,
        _receiver: async_channel::Receiver<Vec<u8>>,
        sender: async_channel::Sender<SocketAction>,
    ) {
        let _ = sender.try_send(SocketAction::Connect(handle, ConnectionState::Failed));
    }
}
