// A release build is a GUI program, like every IDM executable: no console window when it is started from
// a shortcut or at login. Debug builds keep the console for logs.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]

pub mod command_bus;
pub mod commands;
pub mod daemon_session;
pub mod enablement;
pub mod http_client;
pub mod list_io;
pub mod progress_hub;
pub mod single_instance;
pub mod transfer_models;

use cxx_qt_lib::{QGuiApplication, QQmlApplicationEngine, QQuickStyle, QString, QUrl};

fn main() {
    tracing_subscriber::fmt::init();

    if !single_instance::acquire_or_focus_existing() {
        return;
    }

    let mut app = QGuiApplication::new();
    QQuickStyle::set_style(&QString::from("Basic"));
    // The About dialog shows this (Qt.application.version).
    if let Some(app) = app.as_mut() {
        app.set_application_version(&QString::from(env!("CARGO_PKG_VERSION")));
    }
    let mut engine = QQmlApplicationEngine::new();

    if let Some(engine) = engine.as_mut() {
        engine.load(&QUrl::from("qrc:/qt/qml/com/ferric/app/qml/main.qml"));
    }

    if let Some(app) = app.as_mut() {
        app.exec();
    }
    // Leave through the OS instead of unwinding the QML engine and its Rust objects: Qt.quit()
    // used to return from exec() but the process lingered (window gone, ~75 idle threads) while
    // teardown waited on background work.
    std::process::exit(0);
}
