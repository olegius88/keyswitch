//! The settings window on Linux. There is no tray here: closing the window leaves the collector running, starting the
//! program again shows it, and «Выход» on the status page ends it.
use ks_cast::To as _;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};

use crate::catalog::{current_catalog, verify_connection};
use crate::config::{Config, Source, data_directory, load_config, private_directory, save_config};
use crate::constants::{MAX_ROTATIONS, DEFAULT_ROTATIONS, MAX_INTERVAL_MINUTES};
use crate::error::{Error, Result};
use crate::gui_model::{draft, Form, summary, EVENTS_LOG_MAX_LINES, group_label, SOURCE_NOTE, source_from, reset_warning, WINDOW_TITLE_PREFIX, CONNECTION_INTRO, SOURCES_NOTE, delivery_note};
use crate::secrets::{read_token, redact, store_token, token_bot_id};
use crate::service::Service;
use crate::store::Store;
use crate::telegram::Telegram;

const APPLICATION_ID: &str = "io.github.olegius88.LogCourier";

struct Widgets {
    window: gtk::ApplicationWindow,
    notebook: gtk::Notebook,
    token: gtk::PasswordEntry,
    persist: gtk::CheckButton,
    chat: gtk::Entry,
    device: gtk::Entry,
    sources: gtk::ListBox,
    consent: gtk::CheckButton,
    auto: gtk::CheckButton,
    interval: gtk::SpinButton,
    summary: gtk::Label,
    events: gtk::TextView,
    status: gtk::Label,
}

struct App {
    root: PathBuf,
    config: Config,
    sources: Vec<Source>,
    service: Option<Service>,
    widgets: Widgets,
    event_lines: VecDeque<String>,
    last_message: String,
}

type Shared = Rc<RefCell<App>>;

thread_local! {
    static APP: RefCell<Option<Shared>> = const { RefCell::new(None) };
}

fn with_app(action: impl FnOnce(&Shared)) {
    let shared = APP.with(|slot| slot.borrow().clone());
    if let Some(shared) = shared {
        action(&shared);
    }
}

fn say_error(app: &App, message: &str) {
    gtk::AlertDialog::builder().message("LogCourier").detail(redact(message)).build().show(Some(&app.widgets.window));
}

fn draft_of(app: &App) -> Result<(Config, String)> {
    let widgets = &app.widgets;
    draft(
        &app.config,
        &Form {
            token: &widgets.token.text(),
            chat: &widgets.chat.text(),
            device: &widgets.device.text(),
            sources: &app.sources,
            consent: widgets.consent.is_active(),
            auto: widgets.auto.is_active(),
            interval: &widgets.interval.value().to::<i64>().to_string(),
        },
    )
}

fn save(app: &mut App) -> bool {
    let attempt = (|| -> Result<(Config, String)> {
        let (config, token) = draft_of(app)?;
        if !token.is_empty() && app.widgets.persist.is_active() {
            store_token(&token)?;
        }
        save_config(&app.root, &config)?;
        Ok((config, token))
    })();
    match attempt {
        Ok((config, token)) => {
            app.config = config.clone();
            if let Some(service) = &app.service {
                service.update(config, token);
            }
            app.widgets.status.set_text("Настройки сохранены.");
            true
        }
        Err(Error::Os(_)) => {
            say_error(app, "Не удалось сохранить настройки.");
            false
        }
        Err(error) => {
            say_error(app, &error.to_string());
            false
        }
    }
}

fn pause(app: &mut App) {
    app.widgets.auto.set_active(false);
    // Pausing never depends on availability of the system keyring.
    app.config.auto_send = false;
    if let Some(service) = &app.service {
        service.update(app.config.clone(), app.widgets.token.text().trim().to_owned());
    }
    if save_config(&app.root, &app.config).is_err() {
        say_error(app, "Сбор приостановлен, но настройка не сохранена на диск.");
    }
    app.widgets.status.set_text("Автосбор приостановлен. Текущий сетевой запрос может завершиться.");
}

fn refresh_sources(app: &App) {
    while let Some(row) = app.widgets.sources.first_child() {
        app.widgets.sources.remove(&row);
    }
    for source in &app.sources {
        let text = format!("{} — {} (ротаций: {}; старые записи: {})", source.label, source.path, source.rotations, if source.include_existing { "да" } else { "нет" });
        let row = gtk::Label::builder().label(text).xalign(0.0).wrap(true).build();
        app.widgets.sources.append(&row);
    }
}

fn on_status(app: &mut App, message: &str, stats: &Value) {
    let stamp = stats["last_success"].as_f64().and_then(|seconds| glib::DateTime::from_unix_local(seconds.to::<i64>()).ok()).and_then(|at| at.format("%d.%m.%Y %H:%M:%S").ok());
    app.widgets.summary.set_text(&summary(message, stats, &app.sources, stamp.as_deref()).replace('\r', ""));
    if message != app.last_message {
        let clock = glib::DateTime::now_local().ok().and_then(|at| at.format("%H:%M:%S").ok()).map_or_else(String::new, |text| text.to_string());
        app.event_lines.push_back(format!("{clock} {}", redact(message)));
        while app.event_lines.len() > EVENTS_LOG_MAX_LINES {
            app.event_lines.pop_front();
        }
        let all: Vec<&str> = app.event_lines.iter().map(String::as_str).collect();
        app.widgets.events.buffer().set_text(&all.join("\n"));
        message.clone_into(&mut app.last_message);
    }
}

/// A network job off the window's thread; its answer comes back to it.
fn task<T: Send + 'static>(action: impl FnOnce() -> Result<T> + Send + 'static, done: impl FnOnce(&Shared, T) + Send + 'static) {
    std::thread::spawn(move || {
        let result = action();
        glib::MainContext::default().invoke(move || {
            with_app(|shared| match result {
                Ok(value) => done(shared, value),
                Err(error) if error.has_user_text() => say_error(&shared.borrow(), &error.to_string()),
                Err(error) => say_error(&shared.borrow(), &format!("Операция не выполнена ({}).", error.type_name())),
            });
        });
    });
}

fn client_of(shared: &Shared, missing: &str) -> Option<(Telegram, String)> {
    let app = shared.borrow();
    let chat = app.widgets.chat.text().trim().to_owned();
    match Telegram::new(app.widgets.token.text().trim()).and_then(|client| if chat.is_empty() && !missing.is_empty() { Err(Error::value(missing)) } else { Ok(client) }) {
        Ok(client) => Some((client, chat)),
        Err(error) => {
            say_error(&app, &error.to_string());
            None
        }
    }
}

fn find_groups(shared: &Shared) {
    let Some((client, _)) = client_of(shared, "") else { return };
    task(move || client.groups(), |shared, groups: Vec<Value>| {
        let app = shared.borrow();
        if groups.is_empty() {
            return say_error(&app, "Группы не найдены. Добавьте бота и отправьте /start в группе, затем повторите. Либо введите Chat ID вручную.");
        }
        let labels: Vec<String> = groups.iter().map(group_label).collect();
        let dialog = gtk::AlertDialog::builder().message("Группа Telegram").detail("Выберите группу").buttons(labels.iter().map(String::as_str).collect::<Vec<_>>()).build();
        let chat = app.widgets.chat.clone();
        dialog.choose(Some(&app.widgets.window), gtk::gio::Cancellable::NONE, move |choice| {
            if let Some(group) = choice.ok().and_then(|index| groups.get(index.to::<usize>())) {
                chat.set_text(&group["id"].to_string());
            }
        });
    });
}

fn check_connection(shared: &Shared) {
    let Some((client, chat)) = client_of(shared, "Сначала выберите группу.") else { return };
    task(move || verify_connection(&client, &chat), |shared, result: String| {
        let app = shared.borrow();
        gtk::AlertDialog::builder().message("Подключение проверено").detail(result).build().show(Some(&app.widgets.window));
    });
}

fn adopt_catalog(shared: &Shared) {
    let Some((client, chat)) = client_of(shared, "Укажите ID группы.") else { return };
    let bot_id = crate::telegram::Client::bot_id(&client).to_owned();
    let asked = chat.clone();
    task(
        move || {
            verify_connection(&client, &chat)?;
            current_catalog(&client, &chat)
        },
        move |shared, head: Option<Value>| confirm_catalog(shared, head, bot_id, asked),
    );
}

fn confirm_catalog(shared: &Shared, head: Option<Value>, bot_id: String, chat: String) {
    let app = shared.borrow();
    if app.widgets.chat.text().trim() != chat || token_bot_id(app.widgets.token.text().trim()).unwrap_or_default() != bot_id {
        return say_error(&app, "Настройки подключения изменились. Повторите проверку.");
    }
    let Some(head) = head else {
        return say_error(&app, "В группе нет закреплённого каталога LogCourier.");
    };
    let device = head["index"]["device_id"].as_str().unwrap_or_default().to_owned();
    if device == app.config.device_id {
        return say_error(&app, "Этот каталог уже относится к вашему профилю.");
    }
    let dialog = gtk::AlertDialog::builder()
        .message("Перенос каталога")
        .detail("Сначала остановите LogCourier на прежнем компьютере.\nПродолжить его каталог на этом компьютере?\nСтарые логи не скачиваются. Автоотправку потребуется включить отдельно.")
        .buttons(["Да", "Нет"])
        .default_button(1)
        .cancel_button(1)
        .build();
    let weak = shared.clone();
    dialog.choose(Some(&app.widgets.window), gtk::gio::Cancellable::NONE, move |choice| {
        if choice != Ok(0) {
            return;
        }
        let mut app = weak.borrow_mut();
        pause(&mut app);
        match Store::open(&app.root).and_then(|store| store.stats(&format!("{bot_id}:{chat}"))) {
            Ok(stats) if stats["queued"].as_i64().unwrap_or(0) != 0 || stats["unindexed"].as_i64().unwrap_or(0) != 0 => say_error(&app, "Сначала завершите отправку локальной очереди в эту группу."),
            Ok(_) => {
                let old = std::mem::replace(&mut app.config.device_id, device);
                app.widgets.consent.set_active(false);
                if !save(&mut app) {
                    app.config.device_id = old;
                }
            }
            Err(error) => say_error(&app, &error.to_string()),
        }
    });
}

fn add_source(shared: &Shared) {
    let window = shared.borrow().widgets.window.clone();
    let chooser = gtk::FileDialog::builder().title("Выберите лог").build();
    let shared = shared.clone();
    chooser.open(Some(&window), gtk::gio::Cancellable::NONE, move |file| {
        let Some(path) = file.ok().and_then(|file| file.path()) else { return };
        source_dialog(&shared, &path);
    });
}

fn source_dialog(shared: &Shared, path: &Path) {
    let parent = shared.borrow().widgets.window.clone();
    let dialog = gtk::Window::builder().title("Добавить источник").modal(true).transient_for(&parent).build();
    let grid = gtk::Grid::builder().row_spacing(8).column_spacing(8).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
    let name = path.file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    let file = path.display().to_string();
    let (label, rotations) = (gtk::Entry::builder().text(name).hexpand(true).build(), gtk::SpinButton::with_range(0.0, MAX_ROTATIONS.to::<f64>(), 1.0));
    rotations.set_value(DEFAULT_ROTATIONS.to::<f64>());
    let existing = gtk::CheckButton::with_label("Включить уже существующие записи и ротации");
    let note = gtk::Label::builder().label(SOURCE_NOTE.replace('\r', "")).wrap(true).xalign(0.0).build();
    for (row, (title, widget)) in [("Файл", gtk::Label::new(Some(&file)).upcast::<gtk::Widget>()), ("Название", label.clone().upcast()), ("Ротаций", rotations.clone().upcast())].into_iter().enumerate() {
        grid.attach(&gtk::Label::new(Some(title)), 0, row.low::<i32>(), 1, 1);
        grid.attach(&widget, 1, row.low::<i32>(), 1, 1);
    }
    grid.attach(&existing, 1, 3, 1, 1);
    grid.attach(&note, 0, 4, 2, 1);
    let (ok, cancel) = (gtk::Button::with_label("OK"), gtk::Button::with_label("Отмена"));
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.append(&ok);
    buttons.append(&cancel);
    grid.attach(&buttons, 1, 5, 1, 1);
    dialog.set_child(Some(&grid));
    let (closing, shared) = (dialog.clone(), shared.clone());
    cancel.connect_clicked(move |_| closing.close());
    let closing = dialog.clone();
    ok.connect_clicked(move |_| {
        let mut app = shared.borrow_mut();
        match source_from(&file, &label.text(), &rotations.value().to::<i64>().to_string(), existing.is_active(), &app.sources) {
            Ok(source) => {
                app.sources.push(source);
                refresh_sources(&app);
                closing.close();
            }
            Err(error) => say_error(&app, &error.to_string()),
        }
    });
    dialog.present();
}

fn reset_group(shared: &Shared) {
    let app = shared.borrow();
    if app.config.chat_id.is_empty() || app.widgets.token.text().trim().is_empty() {
        return say_error(&app, "Сначала укажите токен и группу на вкладке «Telegram» и сохраните.");
    }
    if app.widgets.chat.text().trim() != app.config.chat_id {
        return say_error(&app, "ID группы изменён, но не сохранён. Сохраните настройки и повторите.");
    }
    let dialog = gtk::AlertDialog::builder()
        .message("Очистить группу")
        .detail(reset_warning(&app.config.chat_id).replace('\r', "").replace("Да — начать заново и удалить сообщения. Нет — начать заново без удаления. Отмена — ничего не менять.", ""))
        .buttons(["Удалить сообщения и начать заново", "Начать заново без удаления", "Отмена"])
        .default_button(2)
        .cancel_button(2)
        .build();
    let weak = shared.clone();
    dialog.choose(Some(&app.widgets.window), gtk::gio::Cancellable::NONE, move |choice| {
        let Ok(index @ (0 | 1)) = choice else { return };
        let app = weak.borrow();
        if let Some(service) = &app.service {
            service.reset(index == 0);
        }
        app.widgets.notebook.set_current_page(Some(3));
        app.widgets.status.set_text("Очистка группы запущена; ход — на вкладке «Состояние».");
    });
}

fn page(notebook: &gtk::Notebook, title: &str) -> gtk::Box {
    let layout = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).margin_top(12).margin_bottom(12).margin_start(12).margin_end(12).build();
    let scroll = gtk::ScrolledWindow::builder().child(&layout).build();
    notebook.append_page(&scroll, Some(&gtk::Label::new(Some(title))));
    layout
}

fn note(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text.replace('\r', "")).wrap(true).xalign(0.0).build()
}

fn form_row(layout: &gtk::Box, title: &str, widget: &impl IsA<gtk::Widget>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let caption = gtk::Label::builder().label(title).xalign(0.0).width_chars(22).build();
    row.append(&caption);
    widget.set_hexpand(true);
    row.append(widget);
    layout.append(&row);
}

fn button_row(layout: &gtk::Box, shared_actions: Vec<(&str, gtk::Button)>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    for (_, button) in &shared_actions {
        row.append(button);
    }
    layout.append(&row);
}

fn action_button(title: &str, action: impl Fn(&Shared) + 'static) -> (&str, gtk::Button) {
    let button = gtk::Button::with_label(title);
    button.connect_clicked(move |_| with_app(|shared| action(shared)));
    (title, button)
}

fn build(application: &gtk::Application, root: PathBuf, config: &Config) -> App {
    let window = gtk::ApplicationWindow::builder().application(application).title(format!("{WINDOW_TITLE_PREFIX} {} — сборщик логов", crate::VERSION)).default_width(850).default_height(640).build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let notebook = gtk::Notebook::builder().vexpand(true).build();
    outer.append(&notebook);
    let status = gtk::Label::builder().label("Отправка только после вашего разрешения").xalign(0.0).margin_start(8).margin_end(8).margin_top(4).margin_bottom(4).build();
    outer.append(&status);
    window.set_child(Some(&outer));
    // Without a tray the window only hides: starting the program again shows it, «Выход» ends it.
    window.set_hide_on_close(true);

    let layout = page(&notebook, "Telegram");
    layout.append(&note(CONNECTION_INTRO));
    let token = gtk::PasswordEntry::builder().show_peek_icon(true).build();
    let persist = gtk::CheckButton::builder().label("Сохранить токен в системном хранилище секретов").active(true).build();
    let chat = gtk::Entry::builder().text(&config.chat_id).placeholder_text("Например, -1001234567890").build();
    let device = gtk::Entry::builder().text(&config.device_name).build();
    form_row(&layout, "Токен", &token);
    layout.append(&persist);
    form_row(&layout, "ID группы (Chat ID)", &chat);
    form_row(&layout, "Имя устройства", &device);
    let save_button = || action_button("Сохранить", |shared| {
        save(&mut shared.borrow_mut());
    });
    button_row(&layout, vec![action_button("Найти группы", find_groups), action_button("Проверить подключение", check_connection), save_button()]);
    layout.append(&action_button("Подключить существующий каталог (перенос с другого компьютера)", adopt_catalog).1);

    let layout = page(&notebook, "Файлы");
    layout.append(&note(SOURCES_NOTE));
    let sources = gtk::ListBox::builder().vexpand(true).selection_mode(gtk::SelectionMode::Multiple).build();
    layout.append(&sources);
    button_row(
        &layout,
        vec![
            action_button("Добавить файл", add_source),
            action_button("Удалить из списка", |shared| {
                let mut app = shared.borrow_mut();
                let mut rows: Vec<usize> = app.widgets.sources.selected_rows().iter().map(|row| row.index().to::<usize>()).collect();
                rows.sort_unstable();
                for row in rows.into_iter().rev() {
                    app.sources.remove(row);
                }
                refresh_sources(&app);
            }),
            save_button(),
        ],
    );

    let layout = page(&notebook, "Отправка");
    let consent = gtk::CheckButton::builder().label("Разрешаю отправлять содержимое выбранных логов в эту группу Telegram").active(config.consent).build();
    let auto = gtk::CheckButton::builder().label("Автоматически собирать и отправлять логи").active(config.auto_send).build();
    layout.append(&consent);
    layout.append(&auto);
    let interval = gtk::SpinButton::with_range(1.0, MAX_INTERVAL_MINUTES.to::<f64>(), 1.0);
    interval.set_value(config.interval_minutes.to::<f64>());
    form_row(&layout, "Интервал отправки, мин", &interval);
    layout.append(&note(&delivery_note()));
    button_row(
        &layout,
        vec![
            save_button(),
            action_button("Отправить сейчас", |shared| {
                let mut app = shared.borrow_mut();
                if !app.widgets.consent.is_active() {
                    return say_error(&app, "Сначала разрешите передачу текста логов на вкладке «Отправка».");
                }
                if save(&mut app)
                    && let Some(service) = &app.service {
                        service.send_now();
                    }
            }),
            action_button("Пауза", |shared| pause(&mut shared.borrow_mut())),
        ],
    );
    layout.append(&action_button("Очистить группу и начать заново…", reset_group).1);

    let layout = page(&notebook, "Состояние");
    let summary_label = gtk::Label::builder().label("Нет отправленных архивов.").wrap(true).xalign(0.0).build();
    layout.append(&summary_label);
    let events = gtk::TextView::builder().editable(false).vexpand(true).build();
    layout.append(&gtk::ScrolledWindow::builder().child(&events).vexpand(true).build());
    layout.append(&note(&format!("Локальные настройки и очередь: {}\nЖурнал состояния не содержит токенов и фрагментов исходных логов.", root.display())));
    let quit_window = window.clone();
    let quit = gtk::Button::with_label("Выход");
    quit.connect_clicked(move |_| {
        with_app(|shared| {
            let app = shared.borrow();
            if let Some(service) = &app.service {
                service.stop();
            }
            app.widgets.status.set_text("Завершение текущего запроса…");
        });
        let application = quit_window.application();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            let mut finished = true;
            with_app(|shared| finished = shared.borrow().service.as_ref().is_none_or(Service::is_finished));
            if finished {
                if let Some(application) = &application {
                    application.quit();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    });
    layout.append(&quit);

    App {
        root,
        config: config.clone(),
        sources: config.sources.clone(),
        service: None,
        widgets: Widgets { window, notebook, token, persist, chat, device, sources, consent, auto, interval, summary: summary_label, events, status },
        event_lines: VecDeque::new(),
        last_message: String::new(),
    }
}

fn start(application: &gtk::Application, root: PathBuf, config: Config, minimized: bool) {
    let mut app = build(application, root.clone(), &config);
    let mut token = std::env::var("LOGCOURIER_BOT_TOKEN").unwrap_or_default();
    if token.is_empty() && !config.bot_id.is_empty() {
        match read_token(&config.bot_id) {
            Ok(found) => token = found,
            Err(error) => app.event_lines.push_back(error.to_string()),
        }
    }
    app.widgets.token.set_text(&token);
    refresh_sources(&app);
    // Another group or token: what was allowed for the old one is no longer allowed.
    let consent = app.widgets.consent.clone();
    app.widgets.chat.connect_changed(move |_| consent.set_active(false));
    let consent = app.widgets.consent.clone();
    app.widgets.token.connect_changed(move |_| consent.set_active(false));
    let notify: crate::service::Notify = Arc::new(|message, stats| {
        glib::MainContext::default().invoke(move || {
            with_app(|shared| on_status(&mut shared.borrow_mut(), &message, &stats));
        });
    });
    app.service = Some(Service::start(root, config.clone(), token, notify));
    let window = app.widgets.window.clone();
    APP.with(|slot| *slot.borrow_mut() = Some(Rc::new(RefCell::new(app))));
    if !minimized || config.bot_id.is_empty() {
        window.present();
    }
}

#[must_use]
pub fn run(minimized: bool) -> i32 {
    let root = data_directory();
    let _ = private_directory(&root);
    let config = if let Ok(config) = load_config(&root) { config } else {
        eprintln!("Не удалось прочитать {}.\nФайл оставлен без изменений; автоматическая отправка не запущена.", root.join("config.json").display());
        return 1;
    };
    let application = gtk::Application::builder().application_id(APPLICATION_ID).build();
    // A second start reaches the first through the application id and shows its window.
    let started = std::cell::Cell::new(false);
    application.connect_activate(move |application| {
        if started.replace(true) {
            with_app(|shared| shared.borrow().widgets.window.present());
            return;
        }
        // The hold keeps the collector going while the window is hidden.
        std::mem::forget(application.hold());
        start(application, root.clone(), config.clone(), minimized);
    });
    i32::from(application.run_with_args::<&str>(&[]).get())
}

/// The window on an isolated profile, built and closed: what an installation check needs to know.
pub fn smoke_test(output: Option<&Path>) -> Result<i32> {
    if gtk::init().is_err() {
        return Err(Error::Runtime("GTK не запустился: нет дисплея.".to_owned()));
    }
    let application = gtk::Application::builder().flags(gtk::gio::ApplicationFlags::NON_UNIQUE).build();
    let app = build(&application, std::env::temp_dir().join("logcourier-self-test"), &Config::default());
    let pages = app.widgets.notebook.n_pages();
    if let Some(output) = output {
        std::fs::write(output, json!({"version": crate::VERSION, "pages": pages, "ok": true}).to_string())?;
    }
    Ok(0)
}
